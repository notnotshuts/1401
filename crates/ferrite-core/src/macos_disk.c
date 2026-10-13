/* Native ownership and geometry only. Formatting and verified file copying stay in Rust. */
#include <CoreFoundation/CoreFoundation.h>
#include <DiskArbitration/DiskArbitration.h>
#include <IOKit/IOBSD.h>
#include <IOKit/IOKitLib.h>
#include <dispatch/dispatch.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/disk.h>
#include <sys/ioctl.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <unistd.h>

int ferrite_media_identity(const char *bsd, uint64_t *id, int *usb, char *serial, size_t capacity) {
    io_service_t media = IOServiceGetMatchingService(0, IOBSDNameMatching(0, 0, bsd));
    if (!media) return ENODEV;
    if (IORegistryEntryGetRegistryEntryID(media, id) != KERN_SUCCESS || !*id) {
        IOObjectRelease(media);
        return ENODEV;
    }
    *usb = 0;
    serial[0] = 0;
    io_registry_entry_t current = media;
    for (unsigned depth = 0; current && depth < 64; depth++) {
        if (IOObjectConformsTo(current, "IOUSBHostDevice") || IOObjectConformsTo(current, "IOUSBDevice")) {
            *usb = 1;
            CFTypeRef value = IORegistryEntryCreateCFProperty(current, CFSTR("USB Serial Number"), NULL, 0);
            if (value) {
                if (CFGetTypeID(value) == CFStringGetTypeID())
                    CFStringGetCString(value, serial, capacity, kCFStringEncodingUTF8);
                CFRelease(value);
            }
            IOObjectRelease(current);
            return 0;
        }
        io_registry_entry_t parent = 0;
        IOReturn result = IORegistryEntryGetParentEntry(current, kIOServicePlane, &parent);
        IOObjectRelease(current);
        current = result == KERN_SUCCESS ? parent : 0;
    }
    if (current) IOObjectRelease(current);
    return 0;
}

struct lease {
    DASessionRef session;
    DADiskRef disk;
    dispatch_queue_t queue;
    dispatch_semaphore_t done;
    int status;
    int claimed;
    char bsd[64];
};

static DADissenterRef deny_release(DADiskRef disk, void *context) {
    (void)disk; (void)context;
    return DADissenterCreate(NULL, kDAReturnBusy, CFSTR("Ferrite is writing this disk"));
}

static DADissenterRef mount_approval(DADiskRef disk, void *context) {
    struct lease *owner = context;
    DADiskRef whole = DADiskCopyWholeDisk(disk);
    const char *name = DADiskGetBSDName(whole ? whole : disk);
    size_t length = strlen(owner->bsd);
    int ours = name && strncmp(name, owner->bsd, length) == 0
        && (name[length] == 0 || name[length] == 's');
    if (whole) CFRelease(whole);
    return ours ? deny_release(disk, context) : NULL;
}

static void completed(DADiskRef disk, DADissenterRef error, void *context) {
    (void)disk;
    struct lease *owner = context;
    owner->status = error ? DADissenterGetStatus(error) : 0;
    dispatch_semaphore_signal(owner->done);
}

static void claimed(DADiskRef disk, DADissenterRef error, void *context) {
    struct lease *owner = context;
    owner->claimed = error == NULL;
    completed(disk, error, context);
}

static void drained(void *context) { (void)context; }

void ferrite_release(void *pointer) {
    struct lease *owner = pointer;
    if (!owner) return;
    if (owner->session && owner->queue) {
        DAUnregisterCallback(owner->session, (void *)mount_approval, owner);
        if (owner->claimed) DADiskUnclaim(owner->disk);
        DASessionSetDispatchQueue(owner->session, NULL);
        dispatch_sync_f(owner->queue, NULL, drained);
    }
    if (owner->disk) CFRelease(owner->disk);
    if (owner->session) CFRelease(owner->session);
    if (owner->done) dispatch_release(owner->done);
    if (owner->queue) dispatch_release(owner->queue);
    free(owner);
}

static int wait_for_disk(struct lease *owner) {
    if (dispatch_semaphore_wait(owner->done, dispatch_time(DISPATCH_TIME_NOW, 30 * NSEC_PER_SEC)))
        return ETIMEDOUT;
    return owner->status;
}

void *ferrite_claim_unmount(const char *bsd, uint64_t expected_id, int *error) {
    struct lease *owner = calloc(1, sizeof(*owner));
    if (!owner) { *error = ENOMEM; return NULL; }
    strlcpy(owner->bsd, bsd, sizeof(owner->bsd));
    owner->queue = dispatch_queue_create("app.ferrite.disk-ownership", DISPATCH_QUEUE_SERIAL);
    owner->done = dispatch_semaphore_create(0);
    owner->session = DASessionCreate(NULL);
    if (!owner->queue || !owner->done || !owner->session) { *error = ENOMEM; goto fail; }
    DASessionSetDispatchQueue(owner->session, owner->queue);
    owner->disk = DADiskCreateFromBSDName(NULL, owner->session, bsd);
    if (!owner->disk) { *error = ENODEV; goto fail; }
    io_service_t media = DADiskCopyIOMedia(owner->disk);
    uint64_t actual = 0;
    int valid = media && IORegistryEntryGetRegistryEntryID(media, &actual) == KERN_SUCCESS;
    if (media) IOObjectRelease(media);
    if (!valid || actual != expected_id) { *error = ENODEV; goto fail; }
    DADiskClaim(owner->disk, 0, deny_release, owner, claimed, owner);
    *error = wait_for_disk(owner);
    if (*error) goto fail;
    DARegisterDiskMountApprovalCallback(owner->session, NULL, mount_approval, owner);
    /* Busy volumes fail; do not force-unmount or terminate another process. */
    DADiskUnmount(owner->disk, kDADiskUnmountOptionWhole, completed, owner);
    *error = wait_for_disk(owner);
    if (*error) goto fail;
    return owner;
fail:
    ferrite_release(owner);
    return NULL;
}

int ferrite_no_mounts(const char *bsd) {
    struct statfs *mounts = NULL;
    int count = getmntinfo(&mounts, MNT_NOWAIT);
    if (count <= 0) return EIO;
    char prefix[80];
    snprintf(prefix, sizeof(prefix), "/dev/%s", bsd);
    size_t length = strlen(prefix);
    for (int i = 0; i < count; i++) {
        const char *name = mounts[i].f_mntfromname;
        if (strncmp(name, prefix, length) == 0 && (name[length] == 0 || name[length] == 's')) return EBUSY;
    }
    return 0;
}

int ferrite_geometry(int fd, uint64_t *bytes, uint32_t *sector) {
    struct stat st;
    uint64_t count = 0;
    uint32_t writable = 0;
    if (fstat(fd, &st)) return errno;
    if (!S_ISBLK(st.st_mode)) return ENOTBLK;
    if (ioctl(fd, DKIOCGETBLOCKSIZE, sector) || ioctl(fd, DKIOCGETBLOCKCOUNT, &count)
        || ioctl(fd, DKIOCISWRITABLE, &writable)) return errno;
    if (!writable || !*sector || count > UINT64_MAX / *sector) return EINVAL;
    *bytes = count * *sector;
    return 0;
}

int ferrite_flush(int fd) {
    /* POSIX fsync drains the block vnode's dirty buffers; F_FULLFSYNC is for regular files. */
    if (fsync(fd)) return errno;
    return ioctl(fd, DKIOCSYNCHRONIZECACHE) ? errno : 0;
}
