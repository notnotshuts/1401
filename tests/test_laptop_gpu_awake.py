"""Laptop NVIDIA GPU gets acpi-wake-type = 1 at its PCI path; desktops and bad paths are untouched."""
import unittest
from types import SimpleNamespace

from p1401 import nullmoth

PATH = "PciRoot(0x0)/Pci(0x1,0x0)/Pci(0x0,0x0)"


def result(platform, path=PATH):
    gpu = {"Manufacturer": "NVIDIA", "Device Type": "Discrete GPU", "Device ID": "10DE-28E0", "PCI Path": path,
           "Compatibility": nullmoth.SEQUOIA}
    return SimpleNamespace(hardware={"Motherboard": {"Platform": platform}, "GPU": {"RTX 4060 Laptop": gpu}}, disabled_devices={})


def cfg():
    return {"NVRAM": {"Add": {}}, "DeviceProperties": {"Add": {}}}


class Awake(unittest.TestCase):
    def run_apply(self, res):
        c, log = cfg(), []
        nullmoth.apply(c, res, lambda *a: log.append(a[0]))
        return c, log

    def test_laptop_gpu_is_kept_awake_at_its_pci_path(self):
        c, log = self.run_apply(result("Laptop"))
        self.assertEqual(c["DeviceProperties"]["Add"][PATH]["acpi-wake-type"], 1)
        self.assertIn("nullmoth-laptop-gpu-awake", log)

    def test_negative_control_a_desktop_is_untouched(self):
        c, log = self.run_apply(result("Desktop"))
        self.assertNotIn(PATH, c["DeviceProperties"]["Add"])
        self.assertNotIn("nullmoth-laptop-gpu-awake", log)

    def test_a_malformed_path_is_never_written(self):
        c, _ = self.run_apply(result("Laptop", path="PciRoot(0x0)/garbage"))
        self.assertEqual(c["DeviceProperties"]["Add"], {})

    def test_panel_warning_handles_linux_connector_types_and_routing(self):
        hardware = result('Laptop').hardware
        hardware['GPU']['Intel UHD'] = {'Compatibility': (None, None)}
        for connector in ('Internal', 'eDP', 'LVDS'):
            with self.subTest(connector=connector):
                panel = {'Connector Type': connector, 'Connected GPU': 'Intel UHD'}
                hardware['Monitor'] = {'Panel': panel}
                self.assertIn('Intel UHD', nullmoth.mux_help(hardware))
                panel['Connected GPU'] = 'RTX 4060 Laptop'
                self.assertIsNone(nullmoth.mux_help(hardware))
        hardware['Monitor']['Panel'] = {'Connector Type': 'HDMI', 'Connected GPU': 'Intel UHD'}
        self.assertIsNone(nullmoth.mux_help(hardware))
        hardware['Monitor']['Panel']['Connector Type'] = 'eDP'
        hardware['GPU']['Intel UHD']['Compatibility'] = ('24.99.99', '19.0.0')
        self.assertIsNone(nullmoth.mux_help(hardware))


if __name__ == "__main__":
    unittest.main()
