# KiCad plugins for stackup KDL

The PCB editor has three stackup actions:

- **Sync PCB from stackup** exports the linked KDL design and updates footprints, pads, and nets. Existing placement and routing are retained.
- **Repeat arrangement** copies selected footprints and connected copper to sibling instances identified by their `Stackup Path` fields.
- **Place at anchor pad** uses `Stackup Anchor`, `Stackup Spot`, `Stackup Pours`, and `Stackup Vias` fields already on footprints. KDL `anchor=host.PIN` on a part exports the host's physical pad to `Stackup Anchor`; optional `spot="dx dy rotation"` exports an exact offset. KDL does not emit pours or vias yet.

## Install

Install the `stackup` KDL CLI first. From this checkout, `cargo install --path crates/stackup` puts it on the shell's `PATH`. A downloaded executable also works. Then run:

```sh
python3 kicad-plugin/install.py
```

The installer chooses the newest KiCad user directory it finds and copies the plugin into its `scripting/plugins/stackup` directory. It records the absolute CLI path for KiCad, whose GUI may have a different `PATH` from the shell. Use `--version 10.0` to select a specific KiCad installation, `--cli /path/to/stackup` to select a CLI, or `--target /path/to/scripting/plugins` for a custom plugin directory. Run it again with `--replace` to update an existing copy. It does not replace another installed `stackup` plugin unless asked.

In KiCad's PCB editor, choose **Tools → External Plugins → Refresh Plugins**, or restart the editor. The three actions appear under External Plugins and on the toolbar. To change the CLI later, rerun the installer with `--replace --cli /path/to/stackup`; `STACKUP_CLI` also overrides the configured path.

For development, a symlink from KiCad's `scripting/plugins/stackup` to this directory lets Python edits appear after **Refresh Plugins**. Keep `stackup` on KiCad's `PATH` or write its absolute path to `stackup-cli.txt` in the plugin directory.

## Link a board

Put a `.stackup_sch` file beside the PCB with the same stem. It contains the path to its KDL design, relative to the PCB directory. For example, `boards/term-01/term-01.stackup_sch` contains:

```text
../../design/term-01/board.kdl
```

The design can import libraries through its `manifest.kdl`; the plugin runs the CLI from the PCB directory and uses the normal manifest resolution. A local `manifest.local.kdl` works as it does at the command line.

To sync a closed board without opening KiCad, use KiCad's bundled Python:

```sh
/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/3.9/bin/python3 \
  kicad-plugin/sync_headless.py boards/term-01/term-01.kicad_pcb
```

The headless command refuses to write a board open in KiCad. The action plugin works on the editor's open board instead; save the board afterward.
