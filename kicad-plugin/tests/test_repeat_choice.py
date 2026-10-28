"""Run with KiCad's Python: python3 -m unittest discover -s kicad-plugin/tests."""
import importlib
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

PLUGIN = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('sync_headless', PLUGIN / 'sync_headless.py')
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)
sync.plugin_modules()
repeat = importlib.import_module('stackup_plugin.repeat')

# One panel slot: a block with no part of its own, everything side by side under `s<n>`.
SLOT = {
    'sw': 'SOT-23-6', 'Cin': 'C_0402', 'Cout': 'C_0402', 'well': 'PinHeader_1x05',
    'pulls/scl': 'R_0402', 'pulls/sda': 'R_0402',
}


class Footprint:
    def __init__(self, path, kind, x, y, selected=False):
        self.path, self.kind, self.selected = path, kind, selected
        self.x, self.y, self.rotation = x, y, 0.0

    def GetReference(self):
        return self.path.upper()

    def IsSelected(self):
        return self.selected

    def HasField(self, name):
        return name == repeat.FIELD

    def GetFieldText(self, name):
        return self.path

    def GetFPID(self):
        kind = self.kind
        return type('FPID', (), {'GetLibItemName': lambda self: kind})()

    def GetPosition(self):
        return type('V', (), {'x': self.x, 'y': self.y})()

    def SetPosition(self, vector):
        self.x, self.y = vector

    def GetOrientationDegrees(self):
        return self.rotation

    def SetOrientationDegrees(self, rotation):
        self.rotation = rotation

    def IsFlipped(self):
        return False

    def GetFields(self):
        return []

    def Pads(self):
        return []


class Board:
    def __init__(self, footprints):
        self.footprints = footprints

    def GetFootprints(self):
        return self.footprints

    def GetTracks(self):
        return []


def slots(count, selected=1):
    """`count` slots 50 units apart; slot `selected` has its parts selected, offset from its well."""
    footprints = []
    for n in range(1, count + 1):
        for i, (name, kind) in enumerate(SLOT.items()):
            x, y = n * 50, 0
            if n == selected and name != 'well':
                x, y = x + 3 + i, 5
            footprints.append(Footprint(f's{n}/{name}', kind, x, y, n == selected))
    return footprints


class RepeatChoiceTest(unittest.TestCase):
    def setUp(self):
        # `_vector` makes a `pcbnew.VECTOR2I`; a tuple is all the fakes need.
        patcher = patch.object(repeat, '_vector', lambda x, y: (x, y))
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_a_slot_with_no_part_of_its_own_has_no_anchor_but_offers_its_parts(self):
        paths = [f's1/{name}' for name in SLOT]
        kinds = {f.path: f.kind for f in slots(3)}
        self.assertIsNone(repeat.anchor_of(paths, kinds))
        self.assertEqual(
            repeat.anchor_choices(paths, kinds),
            ['s1/Cin', 's1/Cout', 's1/pulls/scl', 's1/pulls/sda', 's1/sw', 's1/well'],
        )

    def test_a_selection_across_two_instances_offers_nothing(self):
        kinds = {f.path: f.kind for f in slots(3)}
        self.assertEqual(repeat.anchor_choices(['s1/sw', 's2/Cin'], kinds), [])

    def test_the_chosen_part_is_the_frame_the_others_are_copied_in(self):
        footprints = slots(3)
        offered = []

        def choose(options):
            offered.extend(path for path, _ in options)
            return 's1/well'

        outcome = repeat.repeat(Board(footprints), choose)
        self.assertIn('s1/well', offered)
        self.assertEqual(outcome.anchor, ('s1/well', 'S1/WELL'))
        self.assertEqual(outcome.onto, ['S2/WELL', 'S3/WELL'])
        self.assertEqual(outcome.problems, [])
        at = {f.path: (f.x, f.y) for f in footprints}
        for name in SLOT:
            if name == 'well':
                continue
            dx = at[f's1/{name}'][0] - 50
            self.assertEqual(at[f's2/{name}'], (100 + dx, 5), name)
            self.assertEqual(at[f's3/{name}'], (150 + dx, 5), name)

    def test_closing_the_question_moves_nothing(self):
        footprints = slots(2)
        before = [(f.x, f.y) for f in footprints]
        outcome = repeat.repeat(Board(footprints), lambda options: None)
        self.assertTrue(outcome.cancelled)
        self.assertIsNone(outcome.anchor)
        self.assertEqual([(f.x, f.y) for f in footprints], before)

    def test_without_a_chooser_it_still_reports_no_anchor(self):
        outcome = repeat.repeat(Board(slots(2)))
        self.assertIsNone(outcome.anchor)
        self.assertFalse(outcome.cancelled)


if __name__ == '__main__':
    unittest.main()
