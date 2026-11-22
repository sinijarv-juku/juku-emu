# Juku ROM images

Each file is a complete 16 KB system ROM for the Juku E5101: both 8 KB
EPROMs, 0000–3FFF, in one image. The emulator needs one of them to run.
Choose one in Settings or under Machine → System ROM, where Other… also
takes a 16 KB ROM file of your own. Changing the ROM cold-starts the machine.

| File | Prints at power-on | Notes |
|---|---|---|
| `ekta31.bin` | EktaSoft '88, Serial #0031, RomBios 3.43 | Boots EKDOS |
| `ekta37.bin` | EktaSoft '88, Serial #0037, RomBios 3.43m | **Default.** The ROM the documentation describes; boots EKDOS |
| `jmon33.bin` | MONITOR 3.3 | The older JMON monitor with BASIC built in (`B`). Its disk bootstrap (`T`) is for a WD1791 controller and stops at "Loading system" |

The Key Click setting works with both RomBios ROMs. JMON has no key click
the emulator can change, so the setting is grayed out with it.

- **`ekta37.bin` is the reference.** It is byte-identical to the two halves
  `roms/legacy/jukurom0.bin` and `roms/legacy/jukurom1.bin` in the
  [juku3000](https://github.com/infoaed/juku3000/tree/master/roms) project
  joined together, and to the "3.43m_37" BIOS of MAME's Juku driver. The
  machine description and most tests are based on it.
- **The others are MAME's too.** `ekta31.bin` is the "3.43_31" BIOS and
  `jmon33.bin` the "jmon3.3" BIOS of the same driver, with the same SHA-1.
- **Expansion ROMs are not included.** Machine → Expansion ROM → Other…
  plugs in a file of your own, such as JBASIC 1.1 (`jbasic11.bin`, 8 KB,
  MAME's "expcart"). The emulator accepts a file of up to 32 KB that starts
  with the program header the monitor's `A` command loads. Type `A` at the
  RomBios `*` prompt to start it. Plugging it in or out cold-starts the
  machine.
- **Not covered by this project's license.** The MIT license in
  [LICENSE](../../LICENSE) applies to the emulator's own code and
  documentation. These images were written by others (EktaSoft, EKTA, the
  design bureau of the Institute of Cybernetics in Tallinn), and the rights
  to them remain with their original authors or their successors.
- **Included for preservation.** They are here so that the emulator works out
  of the box and this machine stays usable. If you hold rights to these images
  and want them removed, open an issue in this repository and they will be
  taken out.
