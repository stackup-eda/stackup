"""Run with KiCad's Python: python3 -m unittest discover -s kicad-plugin/tests."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

PLUGIN = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('sync_headless', PLUGIN / 'sync_headless.py')
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)
_, _, apply = sync.plugin_modules()
from stackup_plugin import anchor


class Footprint:
    def __init__(self, ref, target='', position=0):
        self.ref, self.target, self.position = ref, target, position

    def GetReference(self):
        return self.ref

    def IsSelected(self):
        return True

    def HasField(self, name):
        return name == anchor.FIELD and bool(self.target)

    def GetFieldText(self, name):
        return self.target if name == anchor.FIELD else ''


class Board:
    def __init__(self, footprints):
        self.footprints = footprints

    def GetFootprints(self):
        return self.footprints

    def FindFootprintByReference(self, ref):
        return next((f for f in self.footprints if f.ref == ref), None)


def place(footprint, host, pad, slot, offset=None):
    footprint.position = host.position + 1
    return True


class AnchorOrderTest(unittest.TestCase):
    def test_snap_moves_host_before_capacitor_even_in_reverse_board_order(self):
        chip = Footprint('U1', position=100)
        choke = Footprint('L1', 'U1.6')
        capacitor = Footprint('C1', 'L1.2')
        with patch.object(anchor, 'place', side_effect=place):
            result = anchor.snap(Board([capacitor, choke, chip]))
        self.assertEqual(result.moved, ['L1', 'C1'])
        self.assertEqual((choke.position, capacitor.position), (101, 102))
        self.assertFalse(result.problems)

    def test_fresh_sync_places_host_before_dependents(self):
        chip = Footprint('U1', position=100)
        choke = Footprint('L1', 'U1.6')
        capacitor = Footprint('C1', 'L1.2')
        fresh = [(f, SimpleNamespace(ref=f.ref, anchor=f.target, spot=''))
                 for f in [capacitor, choke]]
        outcome = apply.Outcome()
        with patch.object(anchor, 'place', side_effect=place):
            apply._anchor(fresh, {f.ref: f for f in [chip, choke, capacitor]}, {}, outcome)
        self.assertEqual((choke.position, capacitor.position), (101, 102))
        self.assertEqual(outcome.anchored, 2)
        self.assertFalse(outcome.problems)

    def test_unselected_host_remains_where_it_is(self):
        self.assertEqual(anchor.placement_order([('C1', 'L1.2')]), (['C1'], []))

    def test_cycle_and_its_dependents_are_reported_and_stay_put(self):
        a, b, dependent = Footprint('R1', 'R2.1'), Footprint('R2', 'R1.1'), Footprint('C1', 'R1.2')
        with patch.object(anchor, 'place', side_effect=place) as moving:
            result = anchor.snap(Board([a, b, dependent]))
        moving.assert_not_called()
        self.assertTrue(result.problems)
        self.assertFalse(result.moved)

    def test_independent_items_keep_input_order(self):
        self.assertEqual(anchor.placement_order([('C2', 'U1.2'), ('C1', 'U1.1')]), (['C2', 'C1'], []))


if __name__ == '__main__':
    unittest.main()
