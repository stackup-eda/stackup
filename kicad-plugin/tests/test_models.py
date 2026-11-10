"""Run with KiCad's Python: python3 -m unittest discover -s kicad-plugin/tests."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

PLUGIN = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('sync_headless', PLUGIN / 'sync_headless.py')
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)
_, _, apply = sync.plugin_modules()
from stackup_plugin import netlist
import pcbnew


class Models(list):
    def clear(self):
        del self[:]

    def push_back(self, m):
        self.append(m)


class Footprint:
    def __init__(self, ref, *filenames):
        self.ref = ref
        self.models = Models()
        for name in filenames:
            m = pcbnew.FP_3DMODEL()
            m.m_Filename = name
            m.m_Offset = pcbnew.VECTOR3D(1.0, 2.0, 3.0)
            self.models.append(m)

    def GetReference(self):
        return self.ref

    def Models(self):
        return self.models


class Board:
    def __init__(self, *footprints):
        self.footprints = footprints

    def GetFootprints(self):
        return self.footprints


class ModelTest(unittest.TestCase):
    def test_a_project_body_carries_its_source(self):
        [body, stock] = netlist.parse_models(
            'project:stackup.3dshapes/B.step|-7.7 -10.25 0|0 0 0|/lib/stackup.3dshapes/B.step;'
            'kicad:RF_Module.3dshapes/S.step|0 0 0|0 0 90'
        )
        self.assertEqual(body.offset, (-7.7, -10.25, 0.0))
        self.assertEqual(body.source, '/lib/stackup.3dshapes/B.step')
        self.assertEqual((stock.rotate, stock.source), ((0.0, 0.0, 90.0), ''))

    def test_shape_dirs_come_from_the_design_section(self):
        text = '(export (version "E") (design (source "x") (shapes "/a/stackup.3dshapes")) (components))'
        self.assertEqual(netlist.shape_dirs(text), [Path('/a/stackup.3dshapes')])

    def test_a_missing_body_is_found_by_name_and_copied_in(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, library = Path(tmp) / 'board', Path(tmp) / 'lib'
            project.mkdir()
            library.mkdir()
            (library / 'LED.step').write_text('led')
            led = Footprint('D1', '${KICAD99_3DMODEL_DIR}/LED_SMD.3dshapes/LED.step')
            gone = Footprint('J1', '${KICAD99_3DMODEL_DIR}/X.3dshapes/Nowhere.step')
            bare = Footprint('FID1')
            outcome = apply.Outcome()
            apply._check_bodies(Board(led, gone, bare), project, [library], outcome)
            self.assertEqual((project / 'stackup.3dshapes' / 'LED.step').read_text(), 'led')
            [m] = led.Models()
            self.assertEqual(m.m_Filename, '${KIPRJMOD}/stackup.3dshapes/LED.step')
            self.assertEqual(apply._xyz(m.m_Offset), (1.0, 2.0, 3.0))
            self.assertEqual(outcome.found_bodies, ['D1'])
            self.assertEqual(outcome.shipped, ['LED.step'])
            self.assertEqual(outcome.bodiless, ['J1: ${KICAD99_3DMODEL_DIR}/X.3dshapes/Nowhere.step'])

    def test_a_declared_body_is_copied_once_and_only_when_it_changed(self):
        with tempfile.TemporaryDirectory() as tmp:
            project, source = Path(tmp) / 'board', Path(tmp) / 'B.step'
            project.mkdir()
            source.write_text('v1')
            spec = f'project:stackup.3dshapes/B.step|0 0 0|0 0 0|{source}'
            components = {'U1': netlist.Component(ref='U1', models=spec),
                          'U2': netlist.Component(ref='U2', models=spec)}
            outcome = apply.Outcome()
            apply._ship(components, project, outcome)
            self.assertEqual(outcome.shipped, ['B.step'])
            outcome = apply.Outcome()
            apply._ship(components, project, outcome)
            self.assertEqual(outcome.shipped, [])
            source.write_text('v2')
            apply._ship(components, project, outcome)
            self.assertEqual((project / 'stackup.3dshapes' / 'B.step').read_text(), 'v2')


if __name__ == '__main__':
    unittest.main()
