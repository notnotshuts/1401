# Ferrite — Instalación paso a paso

Sigue los pasos en orden. Los nombres de las opciones del BIOS pueden variar en tu PC.

## 1. Comprueba tu hardware

Analiza tu PC o importa Report.json. Elige macOS y revisa las limitaciones de gráficos, red y almacenamiento. Para crear la EFI necesitas una carpeta ACPI con el DSDT del mismo PC.

## 2. Prepara el BIOS

Guarda los ajustes actuales del BIOS. Si Windows usa BitLocker, guarda la clave de recuperación antes de cambiar nada. No cambies RAID/VMD/AHCI sin preparar el sistema instalado.

Marca solo los ajustes que ya hayas comprobado en el BIOS. Estas casillas no modifican el BIOS.

- [ ] UEFI activado; CSM/Legacy desactivado
- [ ] Secure Boot desactivado para arrancar OpenCore
- [ ] Fast Boot desactivado
- [ ] Modo de almacenamiento comprobado: AHCI, VMD desactivado si hace falta
- [ ] XHCI Hand-off activado, si está disponible
- [ ] Above 4G Decoding activado, si está disponible
- [ ] Re-Size BAR revisado para la GPU elegida
- [ ] Intel: CFG Lock, VT-d y DVMT revisados según la configuración; la iGPU suele necesitar al menos 64 MiB de DVMT

## 3. Crea la EFI

Abre OpenCore EFI, elige macOS y la carpeta ACPI. Pulsa «Crear y comprobar EFI». Continúa cuando la validación termine correctamente. Algunos portátiles requieren parches ACPI personalizados.

## 4. Descarga Apple Recovery

Tras crear la EFI, pulsa «Siguiente: descargar Apple Recovery». Se necesita conexión a internet. Espera a que terminen la descarga y la verificación.

## 5. Prepara el USB

Conecta un USB de 8–128 GiB y actualiza la lista. Comprueba modelo, tamaño y número de serie. Guarda los archivos que necesites: la escritura borra todos los datos del USB elegido. Escribe la ruta exacta e inicia la escritura. También puedes crear un archivo de imagen sin modificar las unidades.

## 6. Arranca y comprueba el resultado

Abre el menú de arranque UEFI y elige el USB. Selecciona Recovery en OpenCore; la instalación de macOS usa internet. Comprueba el disco de destino antes de formatearlo. Después, prueba red, audio, USB, pantallas y suspensión. Conserva el USB hasta comprobar el arranque desde el disco interno.

## Después de instalar

- [ ] Ethernet y Wi-Fi
- [ ] Audio y micrófono
- [ ] Puertos USB y dispositivos conectados
- [ ] Todas las pantallas y el control del brillo
- [ ] Suspensión y reactivación
- [ ] Arranque desde la unidad interna

## Si algo sale mal

Error al crear la EFI: comprueba la carpeta ACPI y la ruta de iasl. Falta una unidad: actualiza la lista y consulta el motivo del bloqueo. Error de descarga: comprueba la conexión y vuelve a intentarlo. La EFI no arranca: revisa BIOS y hardware; activa el registro detallado en los ajustes avanzados. Los detalles están en el registro.
