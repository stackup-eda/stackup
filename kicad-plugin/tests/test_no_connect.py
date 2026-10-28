"""Run with KiCad's Python: python3 -m unittest discover -s kicad-plugin/tests."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest

PLUGIN = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('sync_headless', PLUGIN / 'sync_headless.py')
sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sync)
_, _, apply = sync.plugin_modules()
from stackup_plugin import netlist

NETLIST = '''(export (version "E")
  (components
    (comp (ref "U1") (value "chip") (footprint "Test:QFN") (tstamps "u1")))
  (nets
    (net (code "1") (name "link") (class "Default")
      (node (ref "U1") (pin "1") (pinfunction "A") (pintype bidirectional))
      (node (ref "U2") (pin "1") (pinfunction "A") (pintype bidirectional)))
    (net (code "2") (name "unconnected-(U1-B-Pad2)") (class "Default")
      (node (ref "U1") (pin "2") (pinfunction "B") (pintype bidirectional+no_connect)))))
'''


class Pad:
    def __init__(self, number, net='', code=0):
        self.number, self.net, self.code = number, net, code

    def GetNumber(self):
        return self.number

    def GetNetname(self):
        return self.net

    def GetNetCode(self):
        return self.code

    def SetNetCode(self, code):
        self.net, self.code = '', code

    def SetNet(self, net):
        self.net, self.code = net.name, net.code


class Footprint:
    def __init__(self, pads):
        self.pads = pads

    def Pads(self):
        return self.pads


class NoConnectTest(unittest.TestCase):
    def test_parse_records_a_declared_no_connect_as_no_net(self):
        u1 = netlist.parse(NETLIST)['U1']
        self.assertEqual(u1.pads, {'1': 'link', '2': ''})

    def test_wire_clears_a_no_connect_and_keeps_an_undescribed_pad(self):
        component = netlist.parse(NETLIST)['U1']
        link = SimpleNamespace(name='link', code=1)
        # Pad 2 was on the flash clock in an earlier revision; pad 3 is a shield tab the design
        # never mentions.
        pads = [Pad('1'), Pad('2', 'assets/qspi.sck', 7), Pad('3', 'GND', 2)]
        outcome = SimpleNamespace(rewired=0)
        apply._wire(Footprint(pads), component, {'link': link}, outcome)
        self.assertEqual([(p.net, p.code) for p in pads], [('link', 1), ('', 0), ('GND', 2)])
        self.assertEqual(outcome.rewired, 2)


if __name__ == '__main__':
    unittest.main()
