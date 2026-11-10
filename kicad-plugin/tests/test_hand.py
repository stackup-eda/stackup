"""Run with KiCad's Python: python3 -m unittest discover -s kicad-plugin/tests."""
import importlib.util
from pathlib import Path
import unittest

PLUGIN = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('sync_headless', PLUGIN / 'sync_headless.py')
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)
_, _, apply = sync.plugin_modules()
from stackup_plugin import netlist
import pcbnew

POS = pcbnew.FP_EXCLUDE_FROM_POS_FILES
BOM = pcbnew.FP_EXCLUDE_FROM_BOM


def footprint(attrs=0):
    fp = pcbnew.FOOTPRINT(None)
    fp.SetAttributes(pcbnew.FP_SMD | attrs)
    return fp


def has(fp, bits):
    return fp.GetAttributes() & bits == bits


def marker(fp):
    return fp.GetFieldText(apply.HAND_FIELD) if fp.HasField(apply.HAND_FIELD) else ''


class HandParts(unittest.TestCase):
    def test_netlist_reads_the_hand_property(self):
        text = '''(export (version "E")
          (components
            (comp (ref "J1") (value "conn") (footprint "Test:J") (tstamps "j1")
              (property (name "Stackup Hand") (value "yes")))
            (comp (ref "R1") (value "1k") (footprint "Test:R") (tstamps "r1")))
          (nets))'''
        parts = netlist.parse(text)
        self.assertTrue(parts['J1'].hand)
        self.assertFalse(parts['R1'].hand)

    def test_hand_part_is_excluded_and_restored(self):
        fp = footprint()
        self.assertTrue(apply._set_hand(fp, True))
        self.assertTrue(has(fp, POS | BOM))
        self.assertEqual(marker(fp), 'exclude_from_bom exclude_from_pos_files')
        self.assertFalse(apply._set_hand(fp, True))  # a second sync changes nothing
        self.assertTrue(apply._set_hand(fp, False))
        self.assertFalse(fp.GetAttributes() & (POS | BOM))
        self.assertTrue(has(fp, pcbnew.FP_SMD))
        self.assertEqual(marker(fp), '')
        self.assertFalse(apply._set_hand(fp, False))

    def test_land_that_excludes_itself_keeps_its_flags(self):
        # Stock TestPoint and MountingHole lands: `(attr exclude_from_pos_files exclude_from_bom)`.
        fp = footprint(POS | BOM)
        self.assertFalse(apply._set_hand(fp, False))
        self.assertTrue(has(fp, POS | BOM))

    def test_hand_on_a_self_excluding_land_never_clears_it(self):
        fp = footprint(POS | BOM)
        self.assertFalse(apply._set_hand(fp, True))
        self.assertEqual(marker(fp), '')
        self.assertFalse(apply._set_hand(fp, False))
        self.assertTrue(has(fp, POS | BOM))

    def test_only_the_flag_the_sync_added_is_taken_back(self):
        # A stock Fiducial land: `(attr smd exclude_from_bom)`.
        fp = footprint(BOM)
        self.assertTrue(apply._set_hand(fp, True))
        self.assertEqual(marker(fp), 'exclude_from_pos_files')
        self.assertTrue(apply._set_hand(fp, False))
        self.assertFalse(fp.GetAttributes() & POS)
        self.assertTrue(has(fp, BOM))

    def test_flag_cleared_by_hand_is_set_again(self):
        fp = footprint()
        apply._set_hand(fp, True)
        fp.SetAttributes(fp.GetAttributes() & ~POS)
        self.assertTrue(apply._set_hand(fp, True))
        self.assertTrue(has(fp, POS | BOM))
        self.assertEqual(marker(fp), 'exclude_from_bom exclude_from_pos_files')


if __name__ == '__main__':
    unittest.main()
