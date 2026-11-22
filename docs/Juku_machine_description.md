# Juku E5101 — machine description for emulation

This is our best reconstruction of how the machine works, from its ROMs, disks, flux dumps, documents and schematics, and where those say nothing, from MAME's driver and from testing in the emulator. It is not established fact: the tags in brackets say where each statement comes from, §13 lists what is still open, and corrections are welcome.

The Juku is the **E5101 operator console**: the computer with its built-in keyboard. The E5104's passport names it ДГШ3.031.007; the 1987 drawings of the micro-computer "Juku" are ДГШ3.031.006 [BJ][EMM]. Sold with a monitor, a disk unit and optional devices, it formed the **E5104** "intelligent terminal for real-time systems" (ИТСРВ, made by EKTA / SKB VT of the Institute of Cybernetics, Tallinn).

The only CPU module schematic [SCH] (two scans of one drawing) shows an **early board variant** with cassette tape circuitry and no floppy controller. Production boards, the ones emulated here, have the floppy controller in place of the tape USART, and there is no schematic of them. Where only the early drawing exists, its circuits are taken to be unchanged on production boards unless the ROM or the manual shows otherwise. Photos of a production board [PHOTO] show the КР1818ВГ93 floppy controller and a single КР580ВВ51А on the main board, and the same video chips as the drawing. They also show many wires added at the factory, so the wiring differs from the drawing in places. The production board is a later design: the E5101's parts list names its processor module ДГШ5.109.**009**, where the schematic is ДГШ5.109.**006**, and the PROM drawings also name a ДГШ5.109.005 [BJ]. No circuit diagram of either has been found, but the factory's parts lists give the production board's chips (§1, §3.1): the same CPU, bus, timer, PIO and video chips as the drawing, one КР580ВВ51А, the КР1818ВГ93, 8 × К565РУ5Г, two 2764 EPROMs, and none of the tape chips. [BJ] "The 1987 schematic" below means the early variant.

Sources:
- **[SCH]** read from `Schematics/CPU Module.pdf` (ДГШ5.109.006 Э3, "Модуль процессора", 3 sheets, dated 03.1987; also found as `Books/Juku arvuti ES101 joonised - protsessori moodul.pdf`). It shows the early board variant with tape circuitry; see above for how it applies to production boards. A much clearer scan of the same drawing is `Schematics/protsessori_moodul.pdf` [EMM]; the D2 PROM (§3.1) and the video gating around D38/D39 (§7.1) were read from it.
- **[EMM]** read from the 1987 drawing set of the Micro-EVM "Juku" ДГШ3.031.006, scanned by the Estonian electronics museum (https://elektroonikamuuseum.ee) and filed in `Schematics/`: `protsessori_moodul.pdf` (the [SCH] drawing), `emaplaat.pdf` (the module's assembly drawing ДГШ5.109.006 СБ), `jooniste_nimekiri.pdf` (the list of specifications ДГШ3.031.006 ВС), `komponendid.pdf` (the purchased-parts list ДГШ3.031.006 ВП, 11 sheets), `klaviatuur.pdf` (keyboard modules ДГШ5.104.014 and .015: circuit and assembly drawings, key labels), `korpus.pdf` (the machine's assembly drawing ДГШ3.031.006 СБ, case, bottom and board outlines) and `toiteplokk.pdf` (power supply ДГШ5.087.005 / .009). All are signed 03.1987, so they describe the early machine, like [SCH].
- **[KBD]** read from `Schematics/Keyboard.pdf`: keyboard module ДГШ5.104.015. This includes the circuit diagram Э3, the assembly drawing СБ, the parts list ПЭ3, and the technical description ТО with its coding table (Tallinn, 1986, approved 15.08.88).
- **[ROM]** confirmed by disassembling `Roms/jukurom0.bin` + `Roms/jukurom1.bin` ("EktaSoft '88 … RomBios 3.43m").
- **[MAN]** read from `Schematics/User manuals.pdf` and `Schematics/UMan1.pdf` (a cleaner copy of the same manual, "Книга 1", scanned two pages per sheet). The documents are: E5104 operating manual ДГШ3.031.011 РЭ (1988), passport ДГШ3.031.011 ПС (1990) and a disk-copying instruction. The PDF has no text layer; it was OCR'd and the key pages were checked by eye.
- **[FDU]** read from `Schematics/FDDs.pdf`: disk unit Блок НГМД E6502, ДГШ3.065.008. This includes the circuit diagram Э3, the assembly drawing СБ, the parts list ПЭ3 and the specification (approved 15.09.88).
- **[RB]** read from `Schematics/UMan3.pdf`, pages 5–18: "RomBios operator manual" 353872.30024-24 (RomBios v2.4/3.4, EKTA 1987). It covers monitor commands, ROM entry points, system variables, display and keyboard codes, and network functions. Every entry point was checked against the jump table in the supplied ROM (§14).
- **[MOU]** read from `Schematics/Mouse.pdf`: mouse E4701, ДГШ3.049.040. This includes the circuit diagram Э3, assembly drawings, parts lists and the adjustment instruction Д2 (approved 15.08.88). In the scanned circuit diagram the middle strip, between the two halves of the sheet, is missing.
- **[MPCB]** traced from photographs of a production mouse and interface card (both boards marked 04-91) and from Eagle redraws of both boards made from them. The redraws leave out VT1 and some connections, so they were checked against the photos.
- **[GTR]** confirmed by disassembling `Software/gtr.com`, the graphics editor that uses the mouse.
- **[EKD]** read from `Disks/jukprog2/ekdos30.asm`, the source of the 52K EKDOS 2.30 BIOS (A. Kaukver, EktaSoft, last updated 10.04.89). It covers the BIOS only: the BDOS and CCP are CP/M 2.2's, which the BIOS patches in memory.
- **[MAME]** cross-checked against MAME's Juku driver (`src/mame/ussr/juku.cpp`, `jukumouse.cpp` and `lib/formats/juku_dsk.cpp`, master branch, fetched 2026-11-14). MAME's authors had access to real hardware.
- **[PHOTO]** read from photographs of a production board on the Estonian computer museum's Juku page (https://arvutimuuseum.ee/cs00000/, 2019; chips dated 1988–89, board marked N127 by hand). The chip markings are legible; the tracks and the many factory-added wires cannot be followed in them.
- **[BJ]** read from the Baltijets technical documentation for the E5104 (1988–90), found in the former factory in Narva and scanned by the Estonian computer museum in 2024 (`Books/Juku_E5401_Technical_docs/`). It holds the terminal's technical conditions with their acceptance tests (ТУ88 ЭССР 001-89), system-level drawings, parts and spare-parts lists, adjustment instructions and EPROM/PROM programming drawings, but no circuit diagram of the CPU module and no signal or timing checks for the video. One programming table is printed in full: ДГШ5.106.106 Д1, a hex dump of a 2 KB EPROM, which is the first 2 KB of JBASIC 1.1 (§3.2).
- **[FLX]** read from Greaseweazle flux dumps (SuperCard Pro `.scp`, three revolutions a track) of three EKDOS disks from a private collection, `M2.scp`, `M5.scp` and `M6.scp`, decoded with `tools/scpdisk.py`, and from the formatters on the disks: FORMAT.COM 3.4, FMT.COM and FDMAINT.COM. M5 and M6 read without an error, all 1600 sectors on each revolution; M2 is worn, and its readable tracks agree.
- **[INF]** inferred. Treat these as working assumptions.

*Paths to source material (`Schematics/`, `Roms/`, `Software/`) refer to a private archive of Juku documents, ROMs and disks, outside this repository; much of it is public through the juku3000 project (https://github.com/infoaed/juku3000). The same ROM image is in `resources/roms/ekta37.bin`, with RomBios 3.43 #0031 (`ekta31.bin`) and JMON 3.3 (`jmon33.bin`). The disk images are not part of the repository.*

Sheet numbers below are the drawing's own "Лист 1/2/3". In the PDF they appear in the order 3, 2, 1.

---

## 1. Overview

| Item | Value |
|---|---|
| CPU | КР580ВМ80А (Intel 8080A clone), **2 MHz** (16 MHz crystal ÷ 8) [SCH]. The 1987 parts list gives the crystal as РК170ББ-14ГС-16000к [EMM]; the production list has another 16 000 kHz type [BJ]. |
| System controller | КР580ВК38 (8238/8228) [SCH] |
| RAM | 64 KB DRAM: one row of 8 × К565РУ5 (4164), or four rows of 16K×1 chips, К565РУ3 (4116, which also needs +12 V and −5 V) or К565РУ6, selected by jumpers E2/E3 [SCH]. The 1987 parts list fits **four rows of К565РУ3Г, 32 chips**; pin 8 of the RAM sockets is +12 V for them and +5 V for РУ5/РУ6 [EMM]. Production modules ДГШ5.109.009 have **one row of 8 × К565РУ5Г** [BJ]. |
| ROM | 16 KB standard, up to 64 KB, in eight 28-pin sockets D15–D22, which take 2716- to 27128-class EPROMs (jumpers E6/E7). The 1987 parts list fits **eight К573РФ5 (2716, 2 KB)**, programmed as ДГШ5.106.040–047: the eight 2 KB chips of the ROM test (§11). Later ROMs use К573РФ4/РФ6 (2764-class, 8 KB); the production module's parts list has **two M2764AF**, еН5.106.011 and .012, still in eight 28-pin sockets. [SCH][EMM][MAN][BJ] The supplied dump is 2 × 8 KB and is identical to MAME's BIOS **"3.43m_37"** (RomBios 3.43m, "EktaSoft '88 Serial #0037", from the widely sold Baltijets-built E5104 batch). MAME also has JMON 3.3, RomBios 3.42/3.43 variants and tape-capable RomBios 2.43 ROMs. [SCH][MAN][ROM][MAME] |
| Interrupts | КР580ВН59 (8259A), 8080 CALL mode [SCH][ROM] |
| Parallel I/O | 2 × КР580ВВ55 (8255A) [SCH] |
| Serial | 2 × КР580ВВ51 (8251A) in the 1987 schematic: one for RS-232/network, one for cassette tape. Production E5101 units replaced the tape USART with a floppy controller (next row). [SCH][MAN] |
| Floppy | **КР1818ВГ93 (WD1793) on the main board** in production units [MAN][ROM]. The disk unit E6502 (ДГШ3.065.008) holds **two ЕС 5323.01 5.25" double-sided drives in one housing**, daisy-chained on one 34-pin Shugart cable as drives 0 and 1, with its own power supply [FDU]. FM/MFM on 5.25", FM on 8". Standard format: 5.25" MFM, 80 tracks × 2 sides × 10 × 512 bytes = 800 KB (§10.3). |
| Timers | 3 × КР580ВИ53 (8253): video timing, baud rates, sound [SCH][ROM] |
| Video | Monochrome bitmap, linear framebuffer at D800h, 10 KB. Two formats: **320×240** (40×24 text) and **384×200** (64×20 text). The supplied BIOS uses 320×240. Text is drawn in software. 50 Hz composite video, 2 V, sync low, on X6 (VIDEO); the 1987 board also has an RF modulator for a TV set, on X7 (HF). Monitor Электроника МС6105. [ROM][MAN][EMM] |
| Sound | 1-bit beeper from an 8253 output, gated by a PIO bit [SCH][ROM] |
| Expansion | I-41 (Multibus-1 subset) on connector X1 [SCH][MAN]: ADR0–F, DAT0–7, MRDC/MWRC/IORC/IOWC, XACK, INHB, BLOCK, WREQ, INT4/INT6/INT7. It hosts the plug-in memory expander E6201 (≥ 8 KB ROM, typically BASIC; §10.5) and 32 KB external memory pages (§3). |
| Other peripherals | Mouse E4701 (plug-in interface card on the I-41 bus connector X1, port 80h, interrupt IR6; §10.4), printer (СМ6329.02, RAVI-800 [BJ], Epson FX-800 or CPF H80), local network over twisted pair (up to 50 m, ≤ 4800 baud) [MAN] |
| Power | Switch-mode supply ДГШ5.087.005 inside the case, from 220 V mains: +5 V, +12 V and −12 V. Mains switch and 1 A fuse on the rear panel [EMM]. Production units have ДГШ5.087.019 or .020 [BJ]. |
| Rear panel (1987) | From left: X6 VIDEO (coaxial), S1 RESET (push button), X7 HF (coaxial), X5 tape signals (6 pins), X4 tape-deck control (10 pins), X3 RS-232 / network (14 pins), X2 user port, X1 I-41 bus. The base variant labels them in Russian, the -01 variant in English. [EMM][SCH] |
| Rear panel (production) | From left: RESET, VIDEO, X4 disk drives (23 pins, §10.3), X3 RS-232 / network (12 pins in the system diagram), X2, X1, and the mains switch with its fuse. No HF output and no tape connector. Inside, the keyboard plugs into X9 and the power supply into X8. [BJ] |

Logic families on the board: К155 / К555 / К531 TTL (74/74LS/74S), К561 CMOS (CD4000) and К554СА3 (LM311 comparator).

Chip cross-reference: ИЕ7 = 74193, ИЕ10 = 74161, КП14 = 74258, ИД7 = 74138, ИР16 = 74295, ИР82 = 8282, ВА86/ВА87 = 8286/8287, АГ3 = 74123, ТМ2 = 7474, РТ4 = 82S129 (256×4 PROM), РЕ3 = 74S288 (32×8 PROM), 561ИЕ11 = CD4516, 561ИР9 = CD4035, 561ИМ1 = CD4008.

---

## 2. Clocks [SCH]

- **Z1** crystal oscillator at **16 MHz**, built from inverters in D59.
- The D40 counter (74193) and gates D33, D36 and D38 divide 16 MHz into the 8080 two-phase clock: **Φ1/Φ2 = 2 MHz**, plus STB and Φ2TTL. They also produce **2 MHz** and **1 MHz** reference clocks.
- The D103 counter (74161, loaded with 3, counts 13 states) gives 16 MHz ÷ 13 = **1.2308 MHz**. This is the baud-rate reference.
- **VIDEO CYCLE** is 1 MHz: one video DRAM fetch per µs. It also clocks the tape 8251.

---

## 3. Memory map

### 3.1 Decoding hardware [SCH]
- **D6 (РТ4 PROM)** has these inputs: A11–A15, the two memory-map select bits, and a status bit. Its outputs are ROM-enable, RAM-enable, ROE (I/O-decoder enable) and REV.
- **D8 (РЕ3 PROM)** decodes A11–A15 (2 KB granularity) into chip selects for the 8 EPROM sockets.
- **D2 (a second РТ4 PROM)** makes the CPU's READY, not a memory select (§10.3). [EMM]
- The parts list names the three programmed PROMs: КР556РТ4 ДГШ5.106.037 and .038, and К155РЕ3 ДГШ5.106.039. A note on the drawing has D2, D6, D8 and the EPROMs D15–D22 in sockets. [EMM]
- **Production board.** The factory's chip drawings give where each chip is first used. ДГШ5.106.037 and .038 (КР556РТ4, marked with one and two dots) and .040 (the first 2 KB К573РФ5 EPROM) are first used in ДГШ5.109.005. ДГШ5.106.092, marked with four dots, is first used in **ДГШ5.109.009**, the production module. It was drawn as a К573РФ5 EPROM and changed by notice to a **К155РЕ3**. The 1990 spare-parts list gives the production module's programmed chips as ДГШ5.106.037 and .038 (КР556РТ4), ДГШ5.106.092 and еН5.106.010 (two К155РЕ3) and еН5.106.011/.012 (M2764AF): the 1987 board's РЕ3 (.039) is gone and two others take its place. One of them fits a new EPROM select map for the 8 KB EPROMs (16 KB in two chips instead of eight), as the production D8; what the other decodes, perhaps for the floppy controller, is not known. Their programming tables are "на диске" too. [BJ][INF]
- The factory programmed the chips on an Электроника МС 9401 programmer driven by a ДВК-3; its instruction (ДГШ3.031.011 ИП, 1989) lists the types used in the E5104: К155РЕ3, КР556РТ4/РТ5, К573РФ2/РФ5/РФ4 and M2764AFI. [BJ]
- None of the PROMs' contents are available. The map below comes from the user manual (Fig. 3), checked against how the ROM behaves.

### 3.2 Addressing modes: 8255 #1 port C bits 0–1 (port 06h) [MAN][ROM]

The manual (§6.4, Fig. 3) defines four addressing modes ("способ адресации"). **Reads and writes can go to different places**, so model the address space as one 64 KB RAM plus a ROM overlay that applies to reads only.

| PC1..0 | Mode | 0000–3FFF | 4000–BFFF | C000–D7FF | D800–FFFF |
|---|---|---|---|---|---|
| 00 | **0** (after reset) | read **ROM**, write RAM | RAM | RAM | RAM (video), read and write |
| 01 | **1** | RAM | RAM | RAM | read **ROM**, write RAM (video) |
| 10 | **2** | RAM | **external memory window** (32 KB page on the bus) | RAM | read **ROM**, write RAM (video) |
| 11 | **3** | RAM | RAM | RAM | RAM (video), read and write. **No ROM.** |

- **ROM address inside the window.**
  - In mode 0 the ROM appears at its own addresses, 0000–3FFF.
  - In modes 1 and 2, D800–FFFF reads ROM offset `addr & 3FFFh`, which is ROM 1800–3FFF. The manual says the first 6 KB of ROM cannot be reached in these modes.
  - This explains why the code in rom1 (ROM 2000–3FFF) is linked at **E000–FFFF**: it runs in place in mode 1 or mode 2, with no copy needed.
- **Larger ROMs.** The manual notes that a larger ROM moves the ROM/RAM boundary of mode 0 up.
- **Mode 2, the external window.**
  - In mode 2, internal RAM at 4000–BFFF is not selected. Accesses go to an external 32 KB page on the I-41 bus, which can hold ROM or RAM.
  - Several pages are allowed if the expansion hardware can open and close them selectively. The factory's expander E6201 has no page latch and answers in every page (§10.5). The BIOS's page select on PA0–PA2 and PA5 (§5) belongs to its RAM disk, which works in mode 1 and not through this window (§3.5). [BJ][ROM]
  - With nothing plugged in, reads return FFh and writes are ignored. [INF]
  - MAME maps the optional **JBASIC cartridge** (8 KB ROM, v1.1) at 4000h in mode 2, in every page. Monitor command `A` starts it from there (§14.1). [MAME]
  - **The factory's BASIC is JBASIC 1.1.** The programming table of ДГШ5.106.106, a К573РФ2 (2716) first used in ДГШ5.106.103, is printed as a hex dump of 0000–07FFh. It matches the first 2 KB of `jbasic11.bin` wherever the scan is legible: header, the FFh fill from 0010h to 00FFh, code and keyword tables. The one byte that reads differently, 07D0h (C0h, the `RNZ` that starts BASIC's NEW, printed like CAh), is a place where the printer's slashed zero and A look alike in the scan. The chip is the first of the four 2716s on the memory expander E6201 (§10.5). [BJ]
  - **JBASIC 1.1** starts with the program header `JMP 0107h / DW DCBAh / DW 2000h`: key DCBAh, so it does not start at power-on. The `A` command (ROM 1C66h) selects mode 2, takes the length from 4005h, copies that many bytes from 4000h to 0100h and jumps there. At 0107h a `JMP 2000h` leads to a loop that moves 0200–21FF down to 0100–20FF and jumps to 0100h, where BASIC is linked. [ROM][EMU]
  - **JMON 3.3** (`jmon33.bin`, MAME's BIOS "jmon3.3") is a 16 KB system ROM with the same BASIC built in, at ROM 0100h onwards shifted by 2C8h. Its monitor starts BASIC with `B` and the disk bootstrap ("BOOTSTRAP v3.3 - 1791 on Main board") with `T`. The bootstrap was written for a WD1791, whose data bus is inverted compared with the 1793, and stops at "Loading system". [ROM][EMU]
  - Fig. 3 in the manual labels the window's end as 34 KB / 8800h, but its own text says 4000–BFFF (32 KB). Use 4000–BFFF.
- **Mode bits.** The manual's table (legible in UMan1) is: bit pair 00 → mode 0, 01 → mode 1, 10 → mode 2, 11 → mode 3. So the mode number is simply PC1..0.
- **Port number in the manual.** The manual calls this register "the system port, address 02H". Read that as port C, which is register 2 of the 8255. The BIOS uses I/O port **06h** throughout (8255 #1 port C), so emulate 06h.
- **BIOS mode-switch stub.** The BIOS switches modes through a stub in RAM below D800, which stays reachable in every mode:
  - `D7E4: MVI A,1` selects mode 1.
  - `D7E6: MOV H,A` selects the mode passed in A.
  - `D7E7: PUSH PSW / IN 06h / ANI 0FCh / ORA H / OUT 06h / POP PSW / RET` selects the mode passed in H.
- **Mode 2 at boot.** The BIOS uses mode 2 to boot a plug-in ROM: at 1C66h it selects mode 2, reads a header word at 4005h, copies the cartridge from the window to 0100h and jumps there.

### 3.3 RAM areas used by the BIOS [ROM]
- **D800–FDCF**: video framebuffer (the BIOS clears 0x25D0 bytes from D800).
- D300–FFFF is checked by the power-on RAM test. A RAM error shows vertical white lines over the whole screen. [MAN][ROM]
- D300–D7FF: BIOS workspace. Stack is at D450. The warm-boot signature is checked at D75C.
- D75B–D7FF: RAM-resident map-switch and interrupt stubs, copied from ROM 0325h.
- FEC0–FFFF: 8259 vector table and BIOS jump area. In modes 1 and 2 it is read straight from ROM 3EC0h. The BIOS also copies it (0x140 bytes) into RAM for modes 0 and 3.
- The code in rom1 is linked at **E000–FFFF** and runs from ROM in mode 1. For example, the FDC driver at ROM 25xx runs at E5xx.

### 3.4 Memory use under EKDOS (manual appendix) [MAN]
EKDOS is the CP/M-compatible disk operating system.

| Range | Use |
|---|---|
| 0000 | JMP to warm boot |
| 0005 | JMP to BDOS |
| 005C / 006C | FCBs |
| 0080–00FF | DMA buffer |
| 0100–B3FF | TPA (user programs) |
| B400 | CCP |
| BC06–D3FF | BDOS + BIOS |
| D400–D7FF | system parameters |
| **D800–FD7F** | **video memory** (9600 bytes) |
| FD80–FFFF | system parameters |

The appendix also says "ROM: D800–FFFF monitor", which is the mode 1 view.

### 3.5 RAM disk, drive C: [ROM][EKD][EMU]
EKDOS 2.29 and 2.30 offer a 192 KB RAM drive C:, which RomBios serves: FF5C checks for it, and its reads and writes go through the disk BIOS. The ROM's code (ROM 29B3–2AA1, run at E9B3–EAA1) fixes how the card is driven. The card itself is not documented, and the factory's expander E6201 is not it (§10.5).
- **Mode 1 throughout.** The code runs from ROM at E000+ and never touches port 06: under EKDOS the machine is in mode 1 (in the emulator, port 06 holds 05h when FF5C is called). Mode 2's window (§3.2) is not used.
- **Pages.** ROM 2A93h puts page n + 1 on PA0–PA2 with a pulse on PA5 (§5). Pages 1–6 hold the disk. Latching page 7 deselects the card, so the board's RAM is back at 4000–BFFF; page 0 is never latched. [INF]
- **An open page replaces the on-board RAM** at 4000–BFFF: the code reads and writes 4000h in mode 1 and expects the card's bytes there. How the card hides the on-board RAM is not known; the I-41 bus has an inhibit line, INHB (§1). [INF]
- **Transfers** go through a 128-byte buffer at D200h, in four pieces of 32 bytes. For each piece the code disables interrupts, opens the page, copies, latches page 7 and enables interrupts. The buffer is needed because the DMA address in the TPA is hidden while a page is open.
- **Geometry.** 12 tracks of 128 records (16 KB). Track t lies in page t / 2 + 1, at 4000h when t is even and 8000h when it is odd, and record r 80h · r bytes further on. EKDOS's parameters: 1 KB blocks, 192 of them, 64 directory entries in blocks 0–1, no check vector and no system tracks.
- **Detection and formatting** (FF5C, ROM 29B3h):
  1. It reads 4000h with no page open, opens page 1 and compares 12 bytes there with ROM 29A7h, `0F` "RamDisk" `00` and three more. If they match, the disk is kept as it is.
  2. Otherwise it writes the complement of the byte it read to 4000h, latches page 7 and reads 4000h again. If the on-board byte is unchanged, the write went to a card.
  3. It then formats the disk: it clears the first 64 bytes, writes the 12 bytes as the first directory entry, and puts E5h in the other 63 entries. If the on-board byte changed, it returns FFh, and EKDOS leaves C: out of its boot banner.
- **In the emulator** nothing answers at 4000h in another page, so the check fails and EKDOS has no drive C:. [EMU]

---

## 4. I/O map

The 8080 places the port number on both A0–A7 and A8–A15. On this board **the on-board decoding uses A8–A15**:
- **D9** (74138) decodes A10–A12 (port bits 2–4) into CS0–CS7.
- Port bits 5–7 must be 0. D6 checks this and enables D9 through ROE.
- Chip register selects use A8/A9 (port bits 0/1).
- Ports outside 00–1F go to the Multibus.

| Ports | Chip | Notes |
|---|---|---|
| **00–03** | 8259A PIC (D10) | A0 only. 00 = ICW1/OCW2/OCW3, 01 = ICW2/OCW1. 02/03 mirror 00/01. [SCH][ROM] |
| **04–07** | 8255A PIO #1 (D26): keyboard, sound, tape transport, memory map, screen blank | Initialised with control word 82h: PA out, PB in, PC out. [ROM] |
| **08–0B** | 8251A USART (D11): RS-232 / local network | 08 = data, 09 = control/status (0A/0B mirror). The BIOS "NetBios" uses it. The network shares one twisted-pair line between all stations, using the TTL / open-collector SOUT outputs. [ROM][MAN] |
| **0C–0F** | 8255A PIO #2 (D27): **user port** on connector X2 | All 24 lines of PA, PB and PC go to X2 unbuffered, plus two user interrupt inputs IR0/IR1 (X2 pins 14, 18) [MAN]. It is used for the printer (the mouse is on the bus, §10.4). Initialised with 9Bh (all inputs). The printer driver waits for PC3 (port 0E bit 3), then outputs the byte on PA (0C). [ROM] X2 pins as the production manual prints them: PA0–PA7 on 08, 06, 02, 03, 04, 05, 06, 07; PB0–PB7 on 21, 27, 25, 26, 24, 22, 20, 18; PC0–PC7 on 13, 15, 17, 19, 11, 12, 09, 10; IR0 14, IR1 18; +5 V (≤ 0.3 A) 27; GND 29, 30. Pins 06, 27 and 18 appear twice and 01, 16, 23 and 28 not at all, so three entries are misprints (PA1 is probably 01). [BJ] |
| **10–13** | 8253 #1 (D54): horizontal timing | [SCH][ROM] |
| **14–17** | 8253 #2 (D55): vertical timing | [SCH][ROM] |
| **18–1B** | 8253 #3 (D57): baud rates and sound | [SCH][ROM] |
| **1C–1F** | CS7. **Production units:** КР1818ВГ93 / WD1793 FDC, see §10.3 [MAN][ROM]. **1987 schematic:** 8251A USART D93 for the cassette tape (A0 only: 1C/1E data, 1D/1F control) [SCH] | |

Other ports the ROM touches (F0–F3, FB, FF) are off-board Multibus devices, probably a network card. They can be left unconnected (reads return FFh).

---

## 5. 8255 #1 (ports 04–07) signal assignment [SCH][ROM]

**Port A (04h, output)**

| Bit | Signal | Function |
|---|---|---|
| PA0–PA3 | SC0–SC3 | Keyboard column select, 0–15 (decoded on the keyboard) |
| PA4 | AUDC | Speaker **volume**: 1 = loud, 0 = quiet. This is what BLEEPVOL (FFB0h) sets, and key clicks use the quiet level. [MAME][RB] |
| PA5 | — | Not connected on the 1987 schematic. The supplied BIOS uses it as a **strobe that latches PA0–PA2 as the page number for its RAM disk** (§3.5). Routine at ROM 2A93h: `PA = (PA & F8h) \| (n+1) \| 20h`, then clear PA5. The RAM-disk code (E9xxh) opens page n+1 before touching 4000h in mode 1, and latches page 7 to deselect the card. [ROM][MAN][INF] |
| PA6 | PREN (1987) / FDC | 1987 schematic: tape record enable (resets the D97.1 encoder flip-flop on sheet 3). Production FDC: the BIOS sets PA6 = 1 only while it writes the 1793 data register outside a transfer, before a SEEK, then clears it. Most likely PA6 = 1 **disables the DRQ wait-state logic**, so that write does not stall the CPU (§10.3). [ROM][INF] |
| PA7 | STB | Strobe to the keyboard connector X9, pin 10 (board pad 49). The standard keyboard does not use it: its connector X1 has nothing on pin 10. [KBD][BJ] |

The BIOS writes 27h and then 07h to port A at reset.

**Port B (05h, input), active low**

| Bit | Signal | Function |
|---|---|---|
| PB0 | −FK | 0 = some key is pressed in the selected column. This is the 74148 GS output. [KBD] |
| PB1 | K0 | 74148 code output, active low. The ROM reads the row as `(~PB >> 1) & 7`. [KBD][ROM] |
| PB2 | K1 | 〃 |
| PB3 | K2 | 〃 |
| PB4, PB5 | via jumper E8 | PB5 = CONTRDAT: 0 = the DIP switch on the selected column is ON. See §6.4. [KBD][ROM] |
| PB6 | SHIFT | 0 = pressed. Two SHIFT keys, wired in parallel. [KBD] |
| PB7 | CTRL | 0 = pressed [KBD] |

**Port C (06h, output)**

| Bit | Function |
|---|---|
| PC0, PC1 | Memory map select (§3.2), through D3 inverters into D6 |
| PC2 | Production: drive **motor on** / drive enable, set on every disk access. 1987 schematic: tape REC. [ROM][INF] |
| PC3 | Production: **8" mode**. 1987: tape PLAY. [ROM] |
| PC4 | Production: **FM (single density)**; 0 = MFM. 1987: tape FF. [ROM] |
| PC5 | Production: **drive select**, 0 = drive 0 (A:), 1 = drive 1 (B:). 1987: tape RN. [ROM] |
| PC6 | Production: **side select**, 1 = side 1. 1987: tape STOP. [ROM] |

PC2–PC6 go through inverting open-collector drivers to connector **X4**. In production units X4 is the floppy connector (§10.3); on the 1987 board it was the tape deck. The BIOS changes these bits with `IN 06h / ANI 83h / ORA bits / OUT 06h`, which leaves the memory mode (PC0–1) and POF (PC7) untouched.
| PC7 | **POF: picture off**. 1 = video blanked. The BIOS sets PC7 during init and clears it afterwards with `OUT 07h,0Eh`. |

---

## 6. Keyboard [KBD][ROM]

The keyboard module is ДГШ5.104.015, made by the Institute of Cybernetics, Estonian SSR Academy of Sciences. It has 73 keys with Latin, Estonian (Õ Ä Ö Ü) and Cyrillic legends. There is no microcontroller: the CPU scans the matrix directly through 8255 #1.

### 6.1 Hardware
| Part | Function |
|---|---|
| D1, D2 К555ИД7 (74LS138) | Column decoders. **D1** is enabled when SC3 = 0 and drives matrix lines 1–8. **D2** is enabled when SC3 = 1 and drives lines 9–16. The selected line goes low. |
| R1–R6 (4.7 k) | Pull-ups on the 6 matrix rows. A pressed key pulls its row low while its column is selected. |
| D3 К155ИВ1 (74148) | 8→3 priority encoder for the rows. Outputs K0–K2 are active low. −FK (the GS output) is 0 when any row is active. Inputs A6 and A7 are tied high through R9/R10, so they are unused. **The highest-numbered input wins.** |
| S11.1 CTRL, S15.1/S15.2 and S18.2/S18.3 SHIFT | Outside the matrix. Each switch pulls its line to ground, with R7/R8 as pull-ups. |
| S21.1–S21.8 + VD2–VD9 | 8-way DIP switch for keyboard options, read serially on CONTRDAT. See §6.4. |
| VD1 | Power LED |

**Connector X1** (keyboard side, МРН14):

| Pin | Signal |
|---|---|
| 1 | K2 |
| 2 | K0 |
| 3 | K1 |
| 4 | −FK |
| 5 | GND |
| 6 | +5 V |
| 7 | SHIFT |
| 8 | CTRL |
| 9 | CONTRDAT |
| 10 | — |
| 11 | SC0 |
| 12 | SC1 |
| 13 | SC2 |
| 14 | SC3 |

The E5101's wiring diagram (ДГШ3.031.007 Э4) joins this X1 pin for pin to the main board's X9, board pads 58 down to 45. It names the same signals, except that pin 9 is **WAIT** on the main board's side, where the keyboard has CONTRDAT, and pin 10 is STB (PA7, §5). The BIOS reads the DIP switches on PB5 (§6.4), so the name WAIT is probably left over from an earlier use of the line. [BJ][INF]

### 6.2 Encoder rows
D3 sees the matrix rows in this order. "ROM row" is the value the BIOS computes, `r = (~PB >> 1) & 7`, which is the 74148 input number. The priority column says which row is reported if two keys in the same column are down; 6 is the highest.

| ROM row r | K2 K1 K0 on PB3..1 | D3 input | Matrix row (drawing) | Keys | Priority |
|---|---|---|---|---|---|
| 0 | 111 | A0 | 1 | F1–F8 | 1 (lowest) |
| 1 | 110 | A1 | 5 | Z X C … < > ? + | 2 |
| 2 | 101 | A2 | 6 | LAT/RUS ← → SPACE ↑ ↓ DEL | 3 |
| 3 | 100 | A3 | 3 | TAB Q W E … Õ [ ] | 4 |
| 4 | 011 | A4 | 2 | ESC 1 2 … Ю \ ERASE | 5 |
| 5 | 010 | A5 | 4 | CAPS A S D … Ä * RETURN | 6 (highest) |
| 6, 7 | 001, 000 | A6, A7 | — | none (pulled up) | — |

### 6.3 Key matrix: column = PA3..0, row = ROM row

**Use this table in the emulator.** It is what the supplied BIOS (key table at ROM 14D5h) expects. Each cell shows the keycap first, then the BIOS code for unshifted / shifted. A `*` after the code means bit 7 is set in the table: the BIOS uses that flag for letters, which are affected by CAPS and LAT/RUS.

| PA | r0 (F row) | r1 (Z row) | r2 (bottom) | r3 (Q row) | r4 (digits) | r5 (A row) |
|---|---|---|---|---|---|---|
| 0 | F6 → 06h | N → n/N | – | Y → y/Y | 6 → 6/& | H → h/H |
| 1 | – | X → x/X | – | W → w/W | 2 → 2/" | S → s/S |
| 2 | F4 → 18h | V → v/V | – | R → r/R | 4 → 4/$ | F → f/F |
| 3 | F1 → 13h | – | – | TAB → 09h | ESC → 1Bh | CAPS LOCK → 81h |
| 4 | F5 → 01h | B → b/B | – | T → t/T | 5 → 5/% | G → g/G |
| 5 | F2 → 04h | Z → z/Z | – | Q → q/Q | 1 → 1/! | A → a/A |
| 6 | F3 → 05h | C → c/C | – | E → e/E | 3 → 3/# | D → d/D |
| 7 | F7 → 17h | M → m/M | – | U → u/U | 7 → 7/' | J → j/J |
| 8 | – | – | DEL → 7Fh | } ] → ]/} | ERASE → 84h | RETURN → 0Dh |
| 9 | – | – | ↓ → 0Ah | { [ → [/{ | \ ^ → \/^ | – |
| 10 | – | – | ↑ → 0Bh | Õ → }* / \` | Ю @ → @/\` | \* : → :/\* |
| 11 | – | + ; → ;/+ | SPACE → 20h | Ü → ~* / ^ | − = → -/= | Ä → \|* / \\ |
| 12 | – | ? / → //? | → → 0Ch | P → p/P | 0 → 0/_ | Ö → {* / @ |
| 13 | – | > . → ./> | ← → 08h | O → o/O | 9 → 9/) | L → l/L |
| 14 | F8 → 1Ah | < , → ,/< | LAT/RUS → 82h | I → i/I | 8 → 8/( | K → k/K |
| 15 | – | – | – | – | – | – |

- **Wide keys.** TAB, RETURN and SPACE each have two switches on the same contact, so each counts as one key.
- **Unused BIOS codes.** The BIOS table also has entries for positions with no physical key, such as PA 8–13 in row r0. These could be extra keys on a different keyboard variant. Ignore them.
- **Other BIOS table entries.** Each entry also holds an alternate code pair, used when bit 1 of D4BAh (CONSW, RUS mode) is set. The Estonian letters differ there: Õ = `]`/`}`, Ü = `^`/`~`, Ä = `[`/`{`, Ö = `\`/`|`. BIOS rows 6 and 7 hold unrelated data.
- **RUS mode.** For flagged (`*`) entries the BIOS sets bit 7 of the code, giving the KOI-8 Cyrillic letter at C0–FFh (§14.5). For example, A gives E1h = А, and the Õ key's `]` gives DDh = щ. [RB][ROM][INF]

**Columns 8–14 are reversed between the drawing and the BIOS.** In the 1986 drawing and its coding table, D2 is wired straight: PA = 8 + k selects line 9 + k. With that wiring, PA 8 would be the K / I / 8 / < / LAT/RUS / F8 column. The supplied BIOS instead expects the line-15 keys (RETURN / ERASE / ] / DEL) at PA 8, and the whole range 8–14 in reverse. Columns 0–7 agree exactly between the drawing and the BIOS.

The difference is probably a later keyboard PCB revision; the drawing carries a change notice dated 21.08.89. If you emulate exactly what the drawing shows, column `PA` (8–14) moves to `22 − PA`.

### 6.4 Emulating a read of port 05h
```
col  = PA & 0x0F
rows = set of ROM rows r (0..5) with a key down at (col, r)
if rows empty:  PB = 0x01 | 0x0E            ; −FK=1, K=111
else:           r = max(rows)               ; 74148 priority
                PB = ((~r & 7) << 1)        ; −FK=0, K = ~r
PB |= 0x40 if SHIFT not pressed else 0
PB |= 0x80 if CTRL  not pressed else 0
PB |= 0x20 unless (col >= 8 and DIP[col-8] is ON)   ; CONTRDAT
PB |= 0x10 unless (col >= 8 and DIP2[col-8] is ON)  ; PB4: second CONTRDAT bank
```
- **Ghosting.** There are no diodes on the keys, so pressing three keys at the corners of a rectangle produces a phantom fourth key. The BIOS does not handle it, and an emulator can ignore it.
- **Rollover.** The BIOS scans PA = 15 → 0. If keys are down in two different columns it ignores the scan, so there is no n-key rollover.
- **DIP switch S21.** The 8 switches connect through diodes VD2–VD9 to D2's outputs, lines 9–16. The technical description says they are read while SC3 = 1. The switches' common line has a pull-up R12 and reaches CONTRDAT through the series diode VD10, with R13 pulling CONTRDAT down, so it goes **low when the selected column's switch is ON**.
  - At reset the BIOS reads PB5 for PA = 15…8 and stores the bits at D4B9h, with bit (PA − 8) = switch ON. It then shuffles the bit order.
  - **D4B9h is SWSTATE** (system variable (FFA5h) − 5, §14.3), and the switches are the **local-network settings** [RB]:

| SWSTATE bit | Meaning |
|---|---|
| 7 | P: data-transfer coprocessor present |
| 6–5 | MM: network size. 00 = 3 stations, 01 = 7, 10 = 15, 11 = 31 |
| 4–0 | AAAAA: station address |

  - Bit 0 is the rightmost switch and bit 7 the leftmost. The network function SETPARM (FF26h) merges this byte into the network parameter table.
  - Default all switches to OFF (PB5 = 1), which gives SWSTATE = 00h. That means station 0, 3 stations, no coprocessor, which is harmless when no network is emulated.

### 6.5 The Russian keyboard ДГШ5.104.014 [EMM]
The 1987 specification has two variants of the machine. The base variant ДГШ3.031.006 has keyboard module **ДГШ5.104.014** and a case with Russian labels; the variant ДГШ3.031.006-01 has ДГШ5.104.015, the keyboard described above, and English labels.
- **Same circuit.** The 014 module has the same D1/D2 decoders, D3 encoder, S21 switches, matrix and X1 pinout as the 015, drawn with the same column numbering. It is a different board (7.102.094; the 015's is 7.102.093).
- **Different legends.** The keys are laid out ЙЦУКЕН, with the Latin letter that shares each Cyrillic letter's KOI-7 code on the keycap (Й/J, Ц/C, У/U …). The special keys are named in Russian: СП1–СП8, АР2, ГТ, УС, ВК, СБР, ЗБ, ПР, РУС/ЛАТ.
- **The same switch gives a different character.** S3.2, for example, is `1 !` on the 015 and `; +` on the 014, and S7.3 is Q on the 015 and Й J on the 014. The 014 also has one more key in the matrix: S18.2 `_` on line 15 of the Z row, where the 015 has its right SHIFT outside the matrix.
- RomBios 3.43m's table (§6.3) matches the 015's legends, so a machine with the 014 keyboard would need a ROM with its own key table. No such ROM is known. [INF]

---

## 7. Video [SCH][ROM]

### 7.1 Data path
The video circuit is on sheet 2 of the 1987 schematic. It is taken to be the same on production boards, which keep the same video modes, timer ports and ROM video code. A photographed production board has the same video chips, laid out as on the drawing: the two К555ИР16 shift registers beside the КР580ИР82 data latch, the КР531ЛН1 oscillator next to the crystal, four К555ИЕ7 address counters, КР531КП14 address multiplexers and three КР580ВИ53 timers. [SCH][PHOTO][INF]

- **Clocks.** D40 (74193) divides the 16 MHz crystal. Its ÷16 output, 1 MHz, clocks all three counters of T1 (D54) and steps the video address counter, so the timers and the counter cannot drift apart: every change made through T1 falls on a whole byte.
- **Address counter.** D44–D47 (4 × 74193) form a 16-bit video address counter.
  - **Counting:** its count-up input is NAND(1 MHz, HOR RTR) (D39). It steps **once per µs while HOR RTR is high**, whatever VER RTR and POF are doing. It therefore runs on through vertical blank too, 40 bytes for each of the 72 blank lines with the BIOS timing, and through lines blanked by POF.
  - **Loading:** LD of all four counters comes from D34. VER RTR goes through an XOR used as a buffer, a C5 (560 pF) / R33 (620 Ω) differentiator and an XOR used as an inverter, giving a low pulse of about 0.35 µs on the **rising edge of VER RTR**, the end of vertical blank. The falling edge gives no pulse. Nothing else loads the counter.
  - **Loaded value:** A15..A10 from DIP switch S3.1–S3.6 (open = 1, closed = 0; S3.1 = A10, S3.6 = A15), A9..A0 = 0. D44 and D45 have their data inputs grounded.
  - With the BIOS timing the counter is loaded just before the first visible line and steps 40 times per visible line. The framebuffer is therefore linear:
  `addr = base + y*40 + x/8`, with base = **D800h** for the BIOS (S3 = 110110).
- The clearer scan [EMM] confirms the gating: D39 (pins 1, 2 → 3) is the NAND of D40's ÷16 output and HOR RTR, and drives the count-up input of D44; D38 (pins 1, 2, 4, 5 → 6) takes VIDEO CYCLE, HOR RTR, VER RTR and LATCH, and D59 inverts it into LOAD.
- D48–D51 (74258) multiplex the video and CPU addresses onto the DRAM. Video and CPU accesses are interleaved, one video byte per µs (VIDEO CYCLE). VIDEO CYCLE is also an input of the READY PROM D2 (§10.3), so the hardware can hold a CPU access that collides with a video fetch; whether it does, and for how long, depends on D2's unknown contents. The emulator assumes no wait states from video. [EMM][INF]
- **Shift register.** D42 and D43 (2 × 74295) form an 8-bit shift register, shifted at 8 MHz. **The MSB (bit 7) is the leftmost pixel.**
  - It loads a byte only when LOAD is active: LOAD = LATCH · VIDEO CYCLE · HOR RTR · VER RTR (D38, a 4-input NAND, and D59). So a byte is shown only while **both** HOR RTR and VER RTR are high.
  - The serial input of D43 is grounded. When no byte is loaded the register shifts out zeros, so blanking is clean black, and the byte after a gap shows in full.
- 1 = lit pixel, 0 = black. The screen is cleared with 00 and glyph bits are 1. The ROM font confirms bit 7 = leftmost pixel: "F" at ROM 1D02h + 26h·9 renders correctly. [ROM][RB]
- **POF** (PC7 = 1) blanks the output, and nothing else. Through D35 (7406, open collector) it disables the 74295 outputs and pulls the pixel line low. The address counter keeps counting.
  - The BIOS keeps the picture off while its power-on RAM test writes patterns over video RAM (§11), so a good boot shows a dark screen until the banner appears.
  - MAME ignores POF and draws video RAM regardless, so its boot shows the test patterns as vertical stripes. That is a MAME artefact, not the real machine's behaviour. On real hardware, stripes at boot mean a RAM error (§3.3). [MAME]
- The pixel goes through D37 to the video output stage, where the sync signals from D34 are added. Two XOR gates of D34 give SYNC and SIG, which R62/R63 mix into the base of VT2 (КТ315), an emitter follower driving VIDEO (X6). [SCH][EMM]
- **RF output.** The same SYNC and SIG also modulate an oscillator (VT3 КТ315, VT4 КТ325, coil L1) whose output goes through a resistive pad to X7, marked HF, for a TV set. The channel is set by L1 and a trimmer; the drawing does not name it. [EMM]

### 7.2 Timing is set by the BIOS through the 8253s. **The emulated 8253 must support BCD counting.**
Initialisation (ROM 01D4–0220):

| Timer.ch | Clock | Mode / count | Result |
|---|---|---|---|
| T1.0 (10h) | 1 MHz | mode 2, **BCD 64** | Line period **64 µs** (15.625 kHz). OUT0 drives CLK of T2.0 and gates T1.1/T1.2. |
| T1.1 (11h) | 1 MHz | mode 1, BCD 24 | HOR RTR: horizontal blank 24 µs, so **40 µs visible = 40 bytes = 320 px** |
| T1.2 (12h) | 1 MHz | mode 1, BCD 8 | H-sync delay 8 µs. A 74123 one-shot then makes a ~5 µs H-sync. |
| T2.0 (14h) | line clock | mode 2, binary **0139h = 313** | **313 lines/frame → 49.92 Hz** |
| T2.1 (15h) | line clock | mode 1, BCD 72 | VER RTR: vertical blank 72 lines, so **~241 visible lines**. The BIOS uses 240 (24 text rows × 10). |
| T2.2 (16h) | line clock | mode 1, BCD 25 | V-sync delay 25 lines. A 74123 one-shot then makes a ~3–4 line V-sync. |

**Wiring** (sheet 2):

| Signal | Connects to |
|---|---|
| T1 CLK0–2 | 1 MHz (D40) |
| T1 GATE0, T2 GATE0 | +5 V |
| T1 OUT0 | GATE1 and GATE2 of T1; CLK0 of T2 (the line clock) |
| T1 OUT1 | HOR RTR: shift register load, address counter clock |
| T1 OUT2 ("H.SYNC DEL") | B input of D56 half 2 (74123, pin 10; A grounded): the one-shot fires on the **rising** edge. R47 20 kΩ / C7 560 pF, about 5 µs. Q (pin 5) is the horizontal sync; Q̄ (pin 12) clocks T2 CLK1 and CLK2 |
| T2 OUT0 | GATE1 and GATE2 of T2 |
| T2 OUT1 | VER RTR: shift register load, address counter LD pulse (§7.1), and IR5 through the inverter D35 (§7.3) |
| T2 OUT2 ("VERT. SYNC DEL") | B input of D56 half 1 (pin 2; A grounded), rising edge. R59 33 kΩ / C8 15 nF, about 3.5 lines. Q̄ (pin 4) is the vertical sync |

- **T2 CLK1 and CLK2 step when horizontal sync starts.** The 8253 counts on the falling edge of CLK, and the one-shot's Q̄ falls when it fires, on the rising edge of T1 OUT2: 8 µs into the line with the BIOS counts. VER RTR and the vertical sync delay therefore change at that point of a line, inside horizontal blank.
- The sync one-shots fire on rising edges only. The horizontal sync starts T1.2 µs after the line starts, and the picture starts T1.1 µs after it, so the first pixel comes T1.1 − T1.2 µs after the start of sync (16 µs with the BIOS counts).

**Practical emulation:** render 320×240 (or 320×242) from `base = S3<<10`.

**Second format, 384×200 (64×20 text).** The manual specifies it alongside 320×240. It uses the same 9600 bytes, as 48 bytes × 200 lines, and is produced only by reprogramming the 8253s. The supplied BIOS never sets it up. The terminal's acceptance tests check both formats under EKDOS: `MODE 6` for 64 × 20 and `MODE 4` for 40 × 24, each with a test file shown in the SED editor and compared with a printed figure (ТУ88 ЭССР 001-89, §4.11.3). The 64 × 20 figure confirms 20 rows of 64 characters. [BJ]

To support both formats, derive the geometry from the timer counts instead of hard-coding it. MAME does exactly this: it traps writes to ports 10h–12h and 14h–16h, decodes the BCD counts and resizes the visible area. Its default is 320×241. [MAME]

| Quantity | Formula | 320×240 values |
|---|---|---|
| Bytes per line | `T1.0 count − T1.1 count` | 64 − 24 = 40 |
| Visible lines | `T2.0 count − T2.1 count` | 313 − 72 = 241 |
| Frame address | linear from `base` | — |

For 384×200 the counts would give 48 bytes and about 200 lines.

### 7.3 Frame interrupt
VER RTR is inverted (D35, open collector, pulled up by R60) and drives **8259 IR5**. It fires at the start of vertical blanking, every 20.03 ms. The manual calls this the "real-time clock, 20 ms". [SCH][MAN]

### 7.4 The monitor
The Электроника МС6105 is a black-and-white monitor with a near-black bezel. What follows comes from photos and video frames of working and switched-off machines, and from memory. [INF]
- **Phosphor:** white, slightly bluish. Video recordings often show it as cyan; that comes from the camera's white balance, not the tube.
- **Unlit face:** not black. Room light comes back off the grey screen. Video frames put it at 5–13% of full white in a lit room, but the camera brightened them; a few percent looks right. It is the same inside and outside the picture, so it is reflected light, not the beam's black level. Switched off in daylight, the face is a neutral grey with a slight green cast. It darkens to about 60% towards the bezel, in the bezel's shadow.
- **Bezel:** near-black matte plastic, a little lit by the room. Its front stands about 1 cm in front of the edge of the glass. The inner wall leans out from the glass and rounds over into the front with no sharp edge, so a viewer in front of the screen sees it as a band around the glass, about a twentieth of the screen height wide. The wall is lit by the glow of the screen face: dimly by the unlit face, and more where lit picture lies near the edge. Being matte, it only brightens, with no mirror image.
- **Beam bloom:** slight. Solid white areas show no gaps between lines; dithered areas show rows of dots. Overexposed video makes white look bold and dark letters in reverse video thin; a photo of a similar monitor taken at a normal exposure shows one-pixel dark strokes on white staying dark and crisp.
- **White:** video of a working Juku clips it; in photos of similar monitors at a normal exposure it is a slightly bluish white, well short of clipping.
- **Picture size:** the 320-pixel line fills about 78% of the glass width.

---

### 7.5 Changing the timing while the picture is drawn [SCH][INF]
The CPU can write the video timers at any time, and the 2 MHz CPU clock comes from the same crystal as the video: a line is exactly 128 CPU cycles. This section works out, from §7.1–7.2 and the 8253's documented behaviour, what software can do mid-frame. None of it has been tried on a real machine.

- **What a write does.**
  - A new count alone takes effect only at the next period (mode 2, T1.0 and T2.0) or the next trigger (mode 1, the others): for T1, the next line.
  - A control word takes effect at once. The 8253 resets the counter and sets OUT to its initial state: low in mode 0, high in every other mode. The counter then waits for its count.
  - RomBios programs all of T1 as LSB-only BCD (control words 15h, 53h, 93h), so one `OUT` loads a T1 count. T2.1 is programmed for LSB and MSB (73h).
  - The clock and pixel rates cannot be changed: a timer can only count fixed 1 MHz or line pulses differently.
- **Within a line (T1).**
  - A mode 0 control word to T1.1 drops HOR RTR at once: the picture goes black and the address counter stops. A count N written after it raises HOR RTR again N µs later, and the picture continues with the next byte. This makes black gaps of whole bytes that use no memory.
  - A control word for any other mode raises HOR RTR at once, so the picture can start early in a line.
  - Before the line ends T1.1 must be back in mode 1, or the next line's trigger does not start the blank.
  - A control word and count to T1.0 restart the line period, making the current line longer or shorter. That moves the next horizontal sync, and how far the МС6105 follows an irregular line rate is not known.
  - T1.2 is best left alone: its rising edge makes the horizontal sync and also steps the vertical counters.
- **Per line.**
  - **Width and left edge:** a count written to T1.1 sets the next line's blank, so its first byte moves in 8-pixel steps while its right end stays at the end of the line. The line uses 64 − N bytes, which shifts everything below it in memory too.
  - **Blank line, no memory used:** a T1.1 count longer than the line (BCD 99, say) keeps HOR RTR low, since mode 1 is retriggered before it runs out.
  - **Skipped memory:** with POF set and a short blank, a black line uses up to about 60 bytes.
  - **Restart from the base address:** a mode 0 control word to T2.1 drops VER RTR, and a mode 1 control word straight after raises it. The rising edge loads the address counter (§7.1), so the next visible line shows the start of video memory again. Done in horizontal blank, the two `OUT`s do not show. Taking VER RTR low also raises IR5 (§7.3), so a program doing this masks the frame interrupt, and after the last restart of a frame it writes the 72-line count again so that the normal vertical blank still comes.
  - VER RTR cannot be used to skip memory: the counter runs while VER RTR is low, but raising VER RTR again reloads it.
- **Per frame.** The sync delays T1.2 and T2.2 move the whole picture on the tube, horizontally in 8-pixel steps and vertically in lines.
- **Limits.**
  - The address counter only ever goes forward or back to the base address, so a line can show only the next bytes or the start of video memory. Showing an arbitrary row twice, as a general zoom needs, is not possible.
  - Horizontal scrolling finer than 8 pixels has to be done by shifting the bytes in software.
  - One `OUT` takes 10 CPU cycles (5 µs), so a line has room for about seven register writes with nothing else.
- **Finding the beam.** A counter-latch command followed by reading T1.0 gives the position within the line in µs, and the same on T2.0 gives the line. Code can use them to take out interrupt latency before timed writes.

## 8. Interrupts [SCH][ROM]

8259 setup by the BIOS:
- ICW1 = **D6h**: edge-triggered, single, call-address interval 4, vector A7–A5 = 110.
- ICW2 = **FEh**.

The vectors are therefore `CALL FEC0h + 4·n`, and the 8259 supplies a 3-byte CALL during INTA. After init the mask is FFh, then DFh (only IR5 enabled).

| IR | Source |
|---|---|
| 0 | Tape 8251 (D93) RxRDY |
| 1 | Tape 8251 TxRDY |
| 2 | RS-232 8251 (D11) RxRDY |
| 3 | RS-232 8251 TxRDY |
| 4 | TAPE RUN INT (tape-running detector, sheet 3) |
| 5 | **FRAME INT** (vertical blank) |
| 6 | Bus INT6, or tape SYNDET (switch S4) |
| 7 | Bus INT7 |

The 8080 INT pin is driven by the 8259. No NMI.

**How RomBios uses the interrupts** [RB][ROM]:
- The frame interrupt (IR5) is the **system timer**, called "level 8" in the manual. It ticks every 20 ms and its handler takes about 2.5 ms. It scans the keyboard, auto-repeats keys, runs the INTCNT/TIMCNT/network timers and the 10-minute screen saver (PICTON), and handles CTRL+SHIFT hotkeys.
- The timer has low priority. All other levels have high and equal priority.
- Users can hook levels 6 and 7 through INTRSY (FF89h); RomBios 3.4 also allows levels 0, 1 and 4.
- Handlers always run in addressing mode 1.
- The undocumented entry FF7Ah sets (C = FFh) or clears (C = 00h) the 8259 mask bit for level A, updating the shadow copy at D454h. The FDC code uses it to mask IR5 during sector transfers.

---

## 9. Sound [SCH][ROM]

- **T3.1 (port 19h)** is clocked at **2 MHz**, mode 3. Its OUT1 ("SOUND") drives transistor VT1 and the speaker.
- **PA4 sets the volume** rather than gating the sound: speaker level = OUT1 × (PA4 ? 1.0 : 0.5). [MAME][RB]
- Beep frequency = 2 MHz / count. Examples from the BIOS: 08AEh gives 900 Hz, 03E8h gives 2 kHz.
- To silence it the BIOS writes 76h to port 1Bh, which re-arms the counter with its output high.
- Emulate the speaker level as `T3.OUT1 ? (PA4 ? 1.0 : 0.5) : 0`.

---

## 10. Serial, tape, disk

### 10.1 RS-232: 8251 D11, ports 08/09 [SCH][ROM]
- TxC = RxC = T3.0 (port 18h). Clock is 1.2308 MHz, mode 3, **BCD 32**, giving 38.46 kHz = 16 × **2400 baud**.
- The manual says the rate is set in software, from 200 to 9600 baud, asynchronous. [MAN]
- Connector X3 carries the following [MAN]:

| X3 pin | Signal |
|---|---|
| 9 | SOUT (V.24 levels) |
| 3 | TTL SOUT (100 mA) |
| 2, 12 | OC SOUT (open collector, 100 mA) |
| 1 | 120 Ω pull-up to +5 V |
| 10 | RTS |
| 11 | DTR |
| 5 | CTS (in) |
| 6 | DSR (in) |
| 7 | GND |
| 4 | +5 V |

- **Local network:** the TTL/OC outputs let several E5101 stations share one twisted pair. The station-side software is NETOS. Network parameters are set with switches under the label plate on the top panel. These are the keyboard's DIP switch S21, read into SWSTATE (D4B9h) (§6.4). [MAN][RB]
- **Network wiring** (the factory's adjustment instruction, Fig. 1–3) [BJ]:
  - The twisted pair joins X3 pins 7 (GND) and 12 (OC SOUT) of every station.
  - Every station's plug bridges pins 3–4 and 5–10: TTL SOUT to +5 V, and CTS to RTS (our reading of the pin table).
  - Pins 1–2, the 120 Ω pull-up, are bridged only at the two end stations, which terminates the line.
  - The 4-station example sets switches 6–8 to 001, 010, 011 and 100 for stations 1–4, switch 8 being the lowest bit, and switch 2 alike on all four. That agrees with the address in SWSTATE bits 0–4. Which arrow direction is ON is not shown.
  - The test boots a client with `T`, `N` ("Janet 1.2", then "N-EKDOS 1.0 / NET o.k."), from a server running NETD.

### 10.2 Cassette: 8251 D93 at CS7 (1C–1F) [SCH]
- The CLK input is VIDEO CYCLE (1 MHz).
- **T3.2** is clocked at 1.2308 MHz with its gate tied high, and its output, SYNC B., is the tape transmit clock (below). The clearer scan [EMM] shows it plainly: CLK2 on the "1.23M" line, the same as CLK0, and G2 on +5 V. MAME clocks it from VER RTR instead, as a frame counter. What it is wired to on production boards, which have no tape, is not known. [SCH][MAME]
  - **RomBios uses it as a frame counter.** It writes 1Bh ← B0h (channel 2, LSB then MSB, mode 0) and the count FFFFh at start-up (ROM 01FC–020C). Every frame interrupt (ROM 053D) latches it, adds the inverted count, the pulses since the last reload, to the TIMCNT stopwatch at D463h, and reloads FFFEh. On a frame clock that adds 1 a frame, the "20 ms units" of TIMCNT (§14.2), and frames missed while the disk BIOS keeps the interrupt off are still counted. On 1.2308 MHz it adds about 24 600 a frame and the stopwatch wraps every 53 ms. Programs on period disks time themselves with TIMCNT in 20 ms units (PUSHER, LOGER, LAOLEO). [ROM]
  - **RDEM.COM needs it fast.** It builds random numbers from bit 1 of port 1Ah, read nine times in a row without a latch. On a frame clock the counter always holds FFFEh when it reads, every bit is 1, and its random pixels fall on two places. It is remembered working on school machines, which suggests that boards with T3.2 on 1.2308 MHz were in use too, perhaps with another ROM. [EMU][INF]
  - The emulator clocks T3.2 from VER RTR by default and offers 1.2308 MHz as an option. `docs/T3.2_and_TIMCNT.md` has the details.
- **Transmit:** TxC = T3.2 output ("SYNC B.R.", 1.2308 MHz base) ÷ 16 (CD4516, D102). TxD is combined with the bit clock by D97 (JK flip-flops) and an XOR (D95). This is **biphase/Manchester-type encoding**. The result goes to REC.DATA, connector pin 42.
- **Receive:** the LM311 comparator (D106) squares up DATA IN. A digital PLL (D99/D100 CD4035, D101 CD4008 adder) recovers RxC, and an XOR recovers RxD.
- SYNDET goes to IR6 through S4. DTR goes to CNTR CHECK. TAPE RUN goes to IR4.
- Deck transport control uses PC2–PC6 (§5).
- **Emulation:** treat the tape as an 8251 bit stream. Manchester-encode it for WAV output, or attach the decoded bit stream directly at the 8251 for a high-level implementation.

### 10.3 Floppy: КР1818ВГ93 / WD1793 on the main board [MAN][ROM]
The manual (§6.5.2) describes an on-board floppy interface built on the КР1818ВГ93:
- up to 2 drives on a shared bus, or 1 radial;
- FM or MFM on 5.25" disks, FM on 8";
- double-sided.

The ROM reports "DiskBios (Fdc 1793 on MBoard*)" and "BOOTSTRAP v4.1 – 1793 on Main board". It drives the **WD1793 at CS7**, where the 1987 schematic has the tape USART:

| Port | Register |
|---|---|
| 1C | command / status |
| 1D | track |
| 1E | sector |
| 1F | data |

- Data transfers are tight `IN 1Fh` / `OUT 1Fh` loops with no status polling. **The emulator must stall the CPU on a data-register access until DRQ (or INTRQ) is set.** On the real board this happens through the READY logic. On the 1987 board that is the КР556РТ4 PROM **D2**: its inputs include the memory strobes from the system controller, VIDEO CYCLE, and the bus's −XACK and −WREQ; its output D0 is clocked into the flip-flop D30 by Φ2 TTL, whose Q drives the 8080's READY through R29. How the production board brings the FDC's DRQ into this is not known. [SCH][EMM]
- **Drive connector X4** [MAN]. The cable from X4 plugs into the disk unit's socket XS5 (РП15-23, 23 pins), which uses the same pin numbers [FDU]:

| X4 pin | Signal |
|---|---|
| 1–6 | GND |
| 7 | WR.PROTECT |
| 8 | READY |
| 9 | STEP |
| 11 | TG43 |
| 12, 13 | WR.DATA |
| 14 | TR.00 |
| 15 | INDEX |
| 16 | DIR |
| 17 | H.LOAD |
| 18 | WR.GATE |
| 19 | MOTOR ON |
| 20 | S.SELECT (side) |
| 21 | D.SEL1 |
| 22 | D.SEL0 |
| 23 | RD.DATA |

The production manual adds "+5 V, power" to this table without a pin number; pin 10, the only one missing, is the likely place. [BJ][INF]

- **Inside the disk unit E6502** [FDU]:
  - **A1:** power supply ДГШ2.087.034 (the adjustment instruction names ДГШ2.087.031, set to 12.1 V and 5 V) [BJ], giving +5 V and +12 V to both drives. It has a mains switch S1 and a POWER LED VD1.
  - **A2, A3:** two **ЕС 5323.01** floppy drives (5.25", double-sided, 80 tracks per side). They replaced the originally listed К5600.20 / СМ5640.
  - **Cable:** a single 17-wire flat cable runs from XS5 through both drives' 34-pin edge connectors (XS3, XS4). Each drive answers to its own select line, set by a jumper inside the drive.
  - **Signals on the drive connector** (standard Shugart / PC numbering):

| 34-pin | Signal | | 34-pin | Signal |
|---|---|---|---|---|
| 8 | INDEX | | 24 | W.GATE |
| 10 | SEL0 | | 26 | TR0 |
| 12 | SEL1 | | 28 | W.PROT |
| 16 | M.ON | | 30 | RD.DATA |
| 18 | DIR | | 32 | S.SEL (side) |
| 20 | STEP | | 34 | RDY |
| 22 | W.DATA | | odd pins | GND |

  - **Not connected:** X4's TG43 (11) and H.LOAD (17) are not wired in the unit, and neither are pins 12, 13 or 17. The drives load their heads with select/motor, and TG43 is unused.
- **What this means for the emulator** [FDU][ROM][INF]:
  - Emulate **two identical drives**: 5.25", 2 heads, 80 cylinders, 300 rpm (200 ms per revolution; index pulse once per turn).
  - **MOTOR ON is one shared line** for both drives (PC2).
  - The two separate select lines SEL0 and SEL1 must be decoded on the main board from PC5, probably gated by PC2: SEL0 = PC2·¬PC5, SEL1 = PC2·PC5.
  - **READY is wired.** The 1793's NOT READY status bit (bit 7) must reflect whether the selected drive has a disk and its motor running. The BIOS's error mask 9Ch includes that bit.
  - **Head load.** Since H.LOAD is not used, treat the 1793's HLT input as permanently true: no head-load delay is needed.
  - **Write protect** comes from the selected drive (W.PROT); map it to a read-only image flag.
- **Drive control is on 8255 #1 port C** (BIOS routine at E790h / ROM 2790h, which writes through E77Dh):

| PC bit | Meaning when 1 |
|---|---|
| 2 | motor / drive enable (always set during access) |
| 3 | 8" mode |
| 4 | FM |
| 5 | drive 1 |
| 6 | side 1 |

  These map onto the X4 lines MOTOR ON, D.SEL0/1 and S.SELECT. How the 8"/FM bits drive the 1793's DDEN pin and clock was inferred from the BIOS formats. [ROM][INF]
- **Wait states.** MAME halts the CPU on a data-register access during a read or write command while the 1793 is busy and DRQ is low, and releases it when DRQ rises; the 1793 itself is clocked at 1 MHz. Sector transfers are tight `IN 1Fh` / `OUT 1Fh` loops with exact byte counts (128 or 512) and no status polling. The hardware must hold the CPU in WAIT until DRQ.
  - Emulate it this way: a data-register access with PA6 = 0 stalls until DRQ, INTRQ or a timeout.
  - With PA6 = 1 the access goes straight through. The BIOS uses this only to load the target track before SEEK.
  - Completion is then detected by polling the status BUSY bit (E76Fh).
- **Restore.** At start-up the BIOS restores both drives: drive 1 first (PC5 = 1, step rate from D60Fh), then drive 0.
- **Motor.** RomBios never turns PC2 off again once a disk has been used: the motor line stays on while the system idles. An activity indicator must therefore watch the controller (BUSY), not PC2. [ROM][EMU]
- **Verified in the emulator** [EMU]: with the tracks laid out as on real disks (Track format of real disks, below), the supplied ROM boots EKDOS 2.30 ("52K EKDOS 2.30", both drives "5\" 786K"), and SAVE writes files that survive a re-insert. The 6150-byte Read Track and 6161-byte Write Track loops of the BIOS both fit in the 6250-byte track.
- **Track format of real disks** [FLX]. Every track of the three dumped disks has the same layout:
  - MFM at 250 kbit/s and 300 rpm, 6235–6264 bytes a revolution. Cylinders 0–79 are formatted; 80 and 81 are blank.
  - ID fields carry the physical cylinder, the side (0 or 1) and N = 2. The data marks are FB, and a sector that has not been written since formatting holds E5.
  - **The sectors are in the order 1–10, with no interleave.** Sector 1 is at the same place after the index on every track, since Write Track starts each track at the index, so there is no skew from one track to the next either.
  - From the index:

| Bytes | Field |
|---|---|
| 32 × 4E | gap before the index mark |
| C2 C2 C2 FC | index mark, with no 00 bytes before it |
| 32 × 4E | gap 1 |
| 12 × 00, A1 A1 A1 FE, C H R N, CRC | ID field of each sector |
| 22 × 4E | gap 2 |
| 12 × 00, A1 A1 A1 FB, 512 bytes, CRC | data field |
| 24 × 4E, A1 A1 A1 | gap 3; a sector takes 601 bytes in all |
| 103 × 4E | after sector 10 |
| 00 up to the index | 54–74 bytes |

  - The three A1 at the end of each sector are in the formatters' table (below). A 1793 passes over them, since 00 is no address mark.
  - The 00 bytes before the index are the 1793's. The BIOS's Write Track loop ends after 6161 bytes, and the controller writes 00 for each byte the CPU no longer supplies, until the index ends the command.
  - Where a sector has been written since formatting, its new data field starts 22 bytes after the ID field's CRC, as the 1793 writes it.
  - In the dumps the index mark comes 22–31 bytes after the index instead of 32: the index sensors of the drive that formatted a disk and the drive that read it are up to 0.3 ms apart.
- **The formatters** [FLX][ROM]. FORMAT.COM (Ver. 3.4, 786K disks), FMT.COM (386K, СМ5640) and FDMAINT.COM build the track in RAM from the same table of (count, byte) pairs, where F5 writes an A1 sync, F6 a C2 sync and F7 the two CRC bytes:
  - start: `20 4E, 03 F6, 01 FC, 20 4E`;
  - each sector: `0C 00, 03 F5, 01 FE`, C, H, R, N (one byte each), `01 F7, 16 4E, 0C 00, 03 F5, 01 FB, FF E5, FF E5, 02 E5, 01 F7, 18 4E, 03 F5`;
  - end: `FF 4E, FF 4E`;
  - the sector numbers, in track order: `01 02 … 0A` (FDMAINT's table runs on to 1Ah, for 8" disks).

  That is 6058 bytes before the end padding. The BIOS's Write Track (function 22h) loads BC = 1910h but tests only B, so it writes 6161 bytes: the table and 103 bytes of the 4E padding. Its Read Track (21h) likewise reads 1905h − 00FFh = 6150 bytes.
- **Sector order and loading speed** [FLX][EKD][EMU][INF]:
  - EKDOS reads a track in the order 1, 3, 5, 7, 9, 2, 4, 6, 8, 10. The sector two further on comes round about 21 ms after the end of the last one.
  - **In the emulator** EKDOS needs about 24 ms between sectors: it moves four 128-byte records through the BDOS at about 2.5 ms per copy plus overhead. So with the sectors in order it waits a revolution for nearly every sector and loads about 2 KB a second: gtr.com (27 KB) takes 12.5 s.
  - **On the machine this is an assumption, not tested on hardware.** The dumps show only the order of the sectors. The 24 ms is the emulator's timing; a real machine an eighth faster in this code (21 ms instead of 24) would catch the next sector without waiting a revolution, and would load several times faster.
  - The emulator once laid the sectors out as 1, 6, 9, 3, 8, 2, 5, 10, 4, 7, putting each sector of EKDOS's sequence 3 slots after the last, which loaded gtr.com in 5.3 s. Real disks have no such skew.
- The ROM boot loader's table at 255Bh (1, 3, 2, 4, 6, 5, 7, 9, 8, 10) is the order in which it loads the system track. It is a logical-to-sector map, not a physical layout.
- **Logical tracks.** The BIOS numbers tracks across both sides. A logical track ≥ the tracks-per-side value (D610h = 80) selects side 1 with cylinder = track − 80. The 1793 side-compare flag (S = 08h, C = 02h) is set to match.
- **EKDOS file system** on "D" disks [EKD][EMU]. The BIOS source gives the parameters below. Files copied off the images with them match copies made earlier, and juku-sdk's `juku-disk` reads and writes them.
  - **Sector order:** within a logical track, EKDOS's logical sectors 0–9 are sector IDs 1, 3, 5, 7, 9, 2, 4, 6, 8, 10, its reading order (Sector order and loading speed, above). The BIOS's translate table `TRANS` maps the 40 records of 128 bytes in a track to 1–4, 9–12, 17–20, 25–28, 33–36, 5–8, …, four records to a sector. It is not the boot loader's 1, 3, 2, 4, 6, 5, 7, 9, 8, 10 (above). That order reads the same eight sectors for the directory, only in another order, so a reader using it lists the files correctly but scrambles their contents.
  - **Directory:** two system tracks come first (`DIRTRK EQU 2`). Block 0 starts at logical track 2 and holds the directory: 4 KB, 128 entries (`DIRENT EQU 128`).
  - **Blocks and entries:** blocks are 4 KB (`BLKSIZ EQU 4096`), 197 of them (`BLOCKS`), so block numbers are 8-bit, 16 to a directory entry, and an entry covers 64 KB (EXM = 3).
  - **Function numbers:** the BDOS takes functions 0–40 and returns at once for higher numbers (`CPI 29h / RNC`). CP/M 3 calls such as 30h (flush buffers) do nothing.
  - **Single-sided disks:** 400 KB images have the same layout with 2 KB or 4 KB blocks.
- **EKDOS 2.29 can corrupt its CCP during DIR** [EMU][ROM]. Seven of the dumped disks boot "52K EKDOS 2.29", with system tracks identical on all of them (Kryoflux museum001–003 and maalt013, `JUKPROGX.CPM`, both `Juku_9` disks); most others have 2.30. Under 2.29, `DIR` now and then lists the files, then prints text from the BDOS ("BDOS ERR ON : $BAD SECTOR$…") and hangs:
  - The CCP's stack is 16 bytes, directly above its code, which ends with `JMP B782` at BB98–BB9A.
  - Every second file, `DIR` starts a new line, and the call into the BDOS takes SP down to BB9D. For the 13 instructions before the BDOS switches to its own stack, interrupts are on.
  - An interrupt there pushes its return address to BB9B, and RomBios's entry at FED4 (`CALL D79F`) pushes another to BB99, over the jump's address. When the listing ends, the CCP jumps to whatever was left there.
  - The window is about 1.8 ms in a `DIR` of 41 files, against a frame interrupt every 20 ms. In the emulator 23 of 240 `DIR`s on museum001 failed. A warm boot reloads the CCP, so a program run in between repairs it; built-in commands do not.
  - Under 2.30 `DIR` takes SP no lower than BBA1, and no `DIR` failed in 384. The interrupt entry is RomBios code, so a real machine running 2.29 is open to the same failure.
- **Disk formats.** At the boot prompt "System disk type (D/S/8) ?" the BIOS offers three formats:

| Key | Type (D600h) | Medium | Density | Sectors × bytes | Step rate code | Sector interleave |
|---|---|---|---|---|---|---|
| D | 0 | 5.25" DS, 80 tracks/side | MFM (PC4 = 0) | 10 × 512 | 2 | 1,3,2,4,6,5,7,9,8,10 |
| S | 1 | 5.25" | FM (PC4 = 1) | 18 × 128 | 3 | odd sectors, then even |
| 8 | 2 | 8" | FM (PC3 = PC4 = 1) | 26 × 128 (IBM 3740) | 3 | odd sectors, then even |

  The read command is 80h, or 82h with side compare on 5.25". Write is A0h/A2h, Read Track E4h and Write Track (format) F4h. After a transfer the BIOS checks status with mask 9Ch.
- **Emulator disk images.** Provide two drives (A: = drive 0 = SEL0, B: = drive 1 = SEL1). A raw 800 KB image (80 cyl × 2 sides × 10 × 512, sector IDs 1–10) matches the "D" format. MAME's `.juk` format defines the same layout (MFM, 2 µs cells, 80 tracks × 10 × 512, DS 800 KB) plus a single-sided 400 KB variant, and emulates drives as 5.25" 96 tpi ("QD"). The BIOS boot loader reads drive 0 from track 0, loads the system at B000h upwards and jumps to CA00h. [ROM]
- The boot menu reads `System from <D>isk, <N>et ?` and `System disk type (D/S/8) ?`.

Production units have the 1793 at 1C–1F, and the supplied ROM needs it. Put the tape 8251 there only to emulate the early board exactly as drawn, which would also need a tape-era ROM that is not supplied.

---

### 10.4 Mouse E4701 [MOU][MAN]

**Hardware** (manipulator A1, ДГШ5.082.001):
- **Mechanics:** a 25.4 mm ball drives two slotted encoder wheels.
- **Optics:** each axis has one emitter and two receivers. The drawing specifies АЛ107Б for all six (VD5/VD6 emitters, VD1–VD4 receivers). The 1991 unit has two АЛ107Б infrared emitters in series with a 20 Ω resistor, and four 3Л307Б LEDs used as photodetectors. Each receiver feeds a CD4050 input with a 1 MΩ bias resistor and 4.7 MΩ of positive feedback, which makes a Schmitt trigger. [MPCB] The receivers are adjusted for about **90° phase shift**, giving a standard 2-bit quadrature signal per axis: **X1/X2** and **Y1/Y2**.
- **Shaping:** a К561ПУ4 (CD4050) buffer squares up the signals.
- **Buttons:** two, **S1** and **S2** (МП7 microswitches SB1 and SB2). The drawing shows the switches' **break (normally closed) contacts**, terminals 1 and 3. A released switch holds the CD4050 input at 0 V. Pressing the button opens the contact, and a 68 kΩ resistor (R1/R2) pulls the input high, so **pressed = high**. 1.2 nF to 0 V (C1/C2) and 300 kΩ of feedback from the output give some filtering and hysteresis, but not a full debounce. [MOU][MPCB]
- **Resolution:** at least 24 counts per 10 mm, about 61 counts per inch.
- **Cable:** 8 wires (S1, S2, X1, X2, Y1, Y2, +5 V, 0 V) to the interface card; the passport calls the cord 9-core, at least 1 m [BJ]. Both boards number the cable pads 8–16 alike: 8 = S1, 9 = S2, 12/13 = the X phases, 10/11 = the Y phases, 14 = +5 V, 15 and 16 = 0 V. [MPCB]

**Interface card** (A2, ДГШ5.104.023). It plugs into **expansion connector X1** (I-41 bus, plug СНП59-96). The manual's wiring diagram also shows the mouse on X1.

| Part | Chip | Function |
|---|---|---|
| D6, D7 | К555СП1 (74LS85) | Address decoder: compares the bus address with jumpers 1–7 |
| D1, D3, D4 | К561ТМ2 (CD4013) | Three dual flip-flops: the two direction flip-flops, the two step latches, and the two button latches [MPCB] |
| D2 | К561ЛП2 (CD4030) | XOR gates: step detection, and two used as inverters [MPCB] |
| D5 | К555ЛН2 (74LS05) | Open-collector drivers: the buffer enable, interrupt request and XACK |
| D8 | К155ЛН6 (74366) | Inverting tri-state buffer driving **−DAT0…−DAT5** |
| C, R | 1.2 nF, 6.8 kΩ | Differentiator on the select line: clears the latches at the end of each read [MPCB] |
| VT1 | КТ350А (p-n-p) | Drives the latch reset line high while bus −INIT is low (system reset). It is not a one-shot. [MPCB] |

**Address decoding:**
- The card responds **only to I/O reads (−IORC)**. It decodes address lines **A1–A3 and A12–A15**, each against a jumper; A0 and A4–A11 are ignored.
- During an 8080 `IN n` the port number appears on both A0–A7 and A8–A15. So A12–A15 carry port bits 4–7 and A1–A3 carry port bits 1–3.
- The card therefore occupies **two adjacent ports** (A0 ignored). Port bits 1–7 are set by jumpers.
- **Jumpers** [MPCB]. Each jumper is a single hole wired to one comparator input. An open hole leaves the LS input floating high, which matches a high (inactive, so 0) bus line. Closing it to the ground trace beside it matches a 1. Jumpers 1–3 go to −ADR1–3, and 4–7 to −ADRC–F (A12–A15). The comparators also require −IORC low.
- On the photographed card, **jumper 7 is closed by a PCB trace and 1–6 are open**. That decodes A15 = 1 and A1–A3 = A12–A14 = 0: **port 80h/81h**, the address gtr.com uses.
- The card answers with −XACK. Bus pins: −DAT0–5 on C32/B32/C31/B31/C30/B30, −INT6 on C13, −XACK on C6, −IORC on C5, −ADR1/2/3 on B24/C23/B23, −ADRC/D/E/F on C18/B18/C17/B17, −INIT on B1. These match the manual's X1 table.

**Port and data format.** Confirmed by disassembling `Software/gtr.com` (a graphics editor; code at 296Eh, 2AE1h and 2FC9h).
- **Port: 80h.** The card also answers on 81h, since A0 is ignored. That puts jumpers 1–3 (A1–A3) at 0 and A12–A15 at 1000. [GTR] The jumpers on a real card match this. [MPCB]
- **Interrupt:** INT6, **8259 IR6**. The program hooks it with `INTRSY` (A = 6, DE = handler) and unhooks it with DE = 0. It does one `IN 80h` right after hooking to clear any latched state. [GTR]
- **Each interrupt reports one event,** read with a single `IN 80h`, and the read clears the latches. The program's handler just queues the byte in a 256-entry ring buffer; the main loop decodes it later. [GTR][MOU]
- **Clearing happens at the end of the read.** The decoder output both enables D8 and drives a 1.2 nF / 6.8 kΩ differentiator. When the select ends, the pulse resets all four latches (the step and button latches) through their R inputs. A read therefore returns the latched state, and then clears it. [MPCB]

| Bit | Meaning [GTR] |
|---|---|
| 0 | **right** button, **1 = pressed** [MAME] |
| 1 | **left** button, 1 = pressed [MAME] |
| 2 | X direction: 0 = right (X + 1), 1 = left (X − 1) |
| 3 | X step: 1 = the mouse moved one count in X |
| 4 | Y direction: 1 = down (Y + 1), 0 = up (Y − 1) |
| 5 | Y step: 1 = the mouse moved one count in Y |
| 6 | not driven by the card; MAME returns 0 |
| 7 | not driven by the card; MAME returns 1 |

- gtr.com moves the cursor **one pixel per count**. It clamps X to 0–319 and Y to 0–230, because its cursor is 10 pixels tall.
- **How this fits the schematic:**
  - **Per axis** [MPCB]: a direction flip-flop takes phase A on each **rising** edge of phase B, and its Q drives the direction bit directly (bit 2 or 4). It is never cleared, so it always shows the latest direction.
  - The step latch is clocked on each **falling** edge of phase B (through an XOR used as an inverter). Its D input is A XOR (direction Q). If A has changed since B rose, the mouse has moved a full cycle in one direction and the latch sets. If not, the mouse reversed, and clocking in 0 clears any pending step.
  - So there is **one count per full quadrature cycle** (one encoder slot), not one per edge. Each set step latch pulls INT6 and appears as bit 3 or 5.
  - The buttons are buffered straight onto bits 0 and 1: cable S1 → bit 1 (left), S2 → bit 0 (right). The level on the cable is the bit value, so pressed = 1. [MOU][MPCB]
  - D8 drives only −DAT0–5. Bits 6 and 7 read the bus's idle level, and on this card that level should be the same for both.
  - The two button latches have D tied high and are clocked by the button lines. They don't appear in the data byte, but they pull INT6. A CD4013 clocks on the **rising** edge, and a button line rises on **press**. So the hardware requests an interrupt on press only. A release causes one only through contact bounce, or shows up at the next movement event. MAME raises one on both press and release. [MPCB][MAME]
- The ROM has no mouse driver; applications include their own.

**Emulating the mouse:**
```
on host motion (dx, dy in mouse counts; 1 count = 1 pixel in gtr):
    for each unit of |dx|: queue event with bit3=1, bit2=(dx<0)
    for each unit of |dy|: queue event with bit5=1, bit4=(dy>0)   ; +Y = down
on any button change (press or release): queue event with no step bits
pending event -> assert 8259 IR6
IN 80h/81h -> 0x80 | (stepY<<5|dirY<<4|stepX<<3|dirX<<2) | (left<<1) | right, then
              clear the event and deassert IR6 (re-assert if more are queued)
```
- X and Y steps may be combined in one event.
- MAME polls the host mouse at **600 Hz**. Each poll that sees movement or a button change raises IR6, and each read reports one step per axis whatever the size of the host delta. Its authors measured real-hardware rates of **about 440–520 events/s in heavy use, peaking near 1400/s**. Pacing like this keeps the 256-byte ring buffer in gtr.com from overflowing. [MAME]
- **Button polarity.** gtr.com computes `(~byte) & 3` for its button state. The hardware gives pressed = 1 (normally-closed switches, a non-inverting buffer, and two inversions between the cable and the CPU, the same as for the step bits), which agrees with MAME. [MPCB][MAME]

### 10.5 Memory expander E6201 [BJ]

Read from `014 External storage.pdf` in the Baltijets documents, which holds the expander's circuit diagram (ДГШ5.106.103 Э3, two sheets with the strip between them cut off), its assembly drawing and parts list, and from the E5104 passport.

- **What it is.** "Сменный расширитель памяти E6201", ДГШ5.106.102 (approved 15.09.88), is a small box (111 × ~97 × 28 mm) holding one board, ДГШ5.106.103 "Модуль ЗУ-32к", that plugs into X1. The passport gives at least 8 KB of ROM, typically the BASIC interpreter, with the programs "agreed on ordering"; it came only with delivery set No. 1.
- **Variants:**

| Variant | Decoder D4 | D5–D8 | Memory |
|---|---|---|---|
| ДГШ5.106.103 | ДГШ5.106.113 | ДГШ5.106.106–.109, programmed К573РФ2 (2716) | 8 KB ROM: JBASIC 1.1 (§3.2) |
| -01 | ДГШ5.106.117 | 4 × К573РФ4 (2764), unprogrammed in the list | 32 KB ROM |
| -02 | ДГШ5.106.113 | 4 × КР537РУ10 (2K×8 CMOS static RAM) | 8 KB RAM, kept by two Д-0,1 cells |

- **Acceptance test:** plugged into X1 and started with the monitor's `A`, the BASIC variant shows "BASIC / ENSV TA KÜB. I AT EKB / READY". The same text is in the printed dump of its first chip at 04A0h. A variant with test programs, ДГШ5.106.102-04, is listed among the factory's test equipment.
- The E5104's parts lists confirm the base variant: ДГШ5.106.106–.109 are К573РФ2 and ДГШ5.106.113 is a К155РЕ3. The battery, its diode and a switch are struck out there, so the battery-backed RAM variant was probably dropped. The expander and the mouse share connector X1, so a machine has one or the other.

- **Decoding.** D4 is a К155РЕ3 PROM. Its five address inputs are the bus lines −ADRB…−ADRF (A11–A15) straight from the connector, and it is enabled on memory cycles (IO/M). Outputs Q0–Q3 select the four sockets, one each (each chip's OE is tied to its CS), Q4 enables the data transceiver D1 (КР580ВА87, direction from MRDC), and Q5/Q6 appear to drive −XACK. So the card decodes absolute addresses in 2 KB steps. The PROM contents are not given; for JBASIC the 8 KB must sit at 4000–5FFF, where the BIOS's `A` command reads it.
- **No page latch.** There is no register and no use of I/O cycles: the card answers in every page of §3.2, as MAME has it. The PA0–PA2 + PA5 page select (§5), which the BIOS uses for its 192 KB RAM disk (§3.5), is not for this card.
- **−INHIBIT** (X1 pin B6) switches off the address and control buffers D2/D3 (К555АП3) through VT1. The main board presumably releases it only for the mode 2 window, so the card does not collide with internal RAM in other modes. The main board's side of this is not drawn. [INF]
- **Sockets.** Four 28-pin sockets take 2716, 2764 or 537РУ10 chips; a 24-pin chip sits from hole 3. Pin 23 of each goes through a jumper to A11 (2764), +5 V (the 2716's VPP) or WR (RAM write enable), and the chips' supply comes through switch S6 from +5 V or the battery rail. Writes come from bus −AMWTC through VT2 and switch S1, a write-protect switch.
- **X1 pins used:** −D0…−D7 on C32 B32 C31 B31 C30 B30 C29 B29; −ADR0…−ADRF on C24 B24 C23 B23 C22 B22 C21 B21 C20 B20 C19 B19 C18 B18 C17 B17; −IO/M B9, −MRDC C4, −AMWTC B2 (written "B02"), −INHIBIT B6, −XACK C6; +5 V on A6–A8, B8, C8; ground on A1–A4, A24–A27. They agree with the mouse card's pins (§10.4).

### 10.6 Bus connector X1 (I-41) [BJ]
The production manual's table, signals active low:

| Signal | Pins | Notes |
|---|---|---|
| ADR0…ADRF | 24C 24B 23C 23B … 17C 17B | C before B, going down |
| DAT0…DAT7 | 32C 32B 31C 31B 30C 30B 29C 29B | |
| MRDC / MWRC | 4C / 4B | |
| IORC / IOWC | 5C / 5B | |
| AMWC | 2B | advanced memory write (the E6201's AMWTC) |
| IO/M | 9B | 1 = memory |
| XACK | 6C | |
| INHB | 6B | |
| WREQ | 7C | low holds the CPU in wait |
| BLOCK | 7B | input, for diagnostics |
| INIT | 1B | output |
| RESIN | 3C | listed as an output (§12) |
| CCLCK | 11C | CPU clock |
| INT4 / INT6 / INT7 | 14C / 13C / 13B | |
| GND | 1A–3A, 7A–8A | |
| +5 V | 8B, 8C | 0.3 A for cards |
| +12 V | 31A–32A | 50 mA |
| −12 V | 10A–11A | 30 mA (printed "+12") |

## 11. Reset and boot sequence (supplied ROM) [ROM]

1. Reset sets PC = 0000 and map 0. `JMP 0017` falls through an overlapped table to `JMP 01A0`.
2. Configure PIO #2 (9Bh) and PIO #1 (82h). Set PC7 (screen off) and PA = 27h, then 07h.
3. Check the warm-boot signature at D75C. Program the three 8253s (§7.2, §10.1).
4. Run a RAM test over D300–FFFF.
5. Clear the screen (D800, 0x25D0 bytes) and clear PC7 (screen on).
6. Copy the vector area 3EC0 → FEC0 and the map-switch stubs 0325 → D75B. Initialise the 8259 and unmask IR5.
7. Test the ROM. The manual says RomBios prints one `+` per ROM chip (8 in total). A failing chip prints `-` at its position, and the test loops on that chip until reset.
   - Each 2 KB chip is checked with an 8-bit sum (ROM 03CAh, 0426h). Two tables drive it, at ROM 000Bh for chips 0–2 and at 180Bh for chips 3–7. Each table is a length word (1800h, 2800h) with the expected sums stored below it, one byte per chip, going down: 000Ah, 0009h, 0008h and 180Ah … 1806h. Each sum starts just after its table's length word, so neither the tables nor the expected sums are included in any sum. A ROM patch therefore only needs to adjust its own chip's byte. [ROM][EMU]
8. Print the banner ("RomBios 3.4…") and the `*` monitor prompt (§14.1). The factory's acceptance tests expect "RomBios 3.43" after RESET (one reliability programme of 23.8.89 says 3.42), then boot EKDOS from the disk JUKU-1 with `T`, `D`, `D` and run the test QRUN (package DIAGNOSTICS, about 4 minutes, ending "CPU TESTS OK"). [BJ] "A" starts BASIC from a plug-in expander ROM (mode 2). The boot menu offers `System from <D>isk, <N>et ?`.
9. Load the system, then jump to the D7E4 stub. It selects **mode 1** (RAM 0000–D7FF, ROM readable at D800–FFFF) and continues at FEEAh (= ROM 3EEAh), or at 0100h for a loaded program. [ROM][MAN]

---

## 12. Jumpers and switches relevant to emulation [SCH]

| Ref | Function |
|---|---|
| S1 | Reset push button (КМ1-1) on the rear panel. It discharges C1 (47 µF, charged through R2 20 kΩ), which drives RES through D13. The same node is brought out to the bus as −RESIN (X1 pin 3C), which the production manual lists as an output. [EMM][BJ] |
| S3.1–S3.6 | Video base address A10–A15 (a 6-way DIP switch, ВДМ1-6). Default for this ROM: D800h. In the E5101's 1988 parts list the ВДМ1-6 for ДГШ5.109.009 is struck out, so production boards probably have the base fixed (wired) instead. [BJ][INF] |
| S4 | IR6 source: bus INT6 or SYNDET (a 2-way DIP switch, ВДМ1-2) |
| E2, E3 | RAM type: one row of 4164, or 4 rows of 16K chips decoded by A14/A15 |
| E6, E7 | EPROM pin 27 option (2764 / 27128) |
| E8 | PB4/PB5 keyboard option lines |
| E10, E13, E14, E1 | Clock/encoder options. They do not matter for emulation. |

---

## 13. Open points

- The addressing modes and their bit values are now settled by the manual (§3.2). The manual says Fig. 3 applies to 16 KB ROM only: with 32 or 64 KB ROM the zone boundaries move but keep the same order. The exact boundaries for those variants would need a dump of the D6 PROM. On the photographed production board both decoding PROMs, К155РЕ3 (D8) and К556РТ4А (D6), sit in sockets, so they could be read from a board. [PHOTO] Their programming tables (ДГШ5.106.037 Д1 for the КР556РТ4) are marked "на диске", on a disk that is not in the Baltijets documents. [BJ]
- Settled by MAME: PA4 is the volume select; pixels are bit 7 = left and 1 = lit (white); the keyboard columns follow the BIOS order (§6.3); and the mouse's event rate is known. MAME interrupts on button release too; the traced card does so only through contact bounce (§10.4). The mouse card's jumpers, latch clearing and button polarity are settled by the traced boards [MPCB].
- The keyboard's columns 8–14 are reversed between the 1986 drawing and the BIOS (§6.3). MAME's keyboard matrix matches the BIOS order, so production keyboards are wired that way. The factory's keyboard parts list has a variant ДГШ5.104.015-01 with its own board, ДГШ7.102.132, documented only on data media that are not in the scans; the production E5101 uses the -01, so its board is the likely home of the reversed columns. [BJ][INF] The meaning of the S21 switches is now known (network settings, §6.4); only which physical switch sits on which column is undocumented.
- The manual's EXTPG entry address ("FEBDH") is a misprint, and the supplied 3.43m ROM has no public EXTPG entry (§14.2).
- There is no schematic of the production FDC section of the main board; the disk unit side is documented (§10.3). The drive-control bits (PC2–PC6) come from the BIOS. MAME confirms PC2 = motor, PC4 = density (FM = 1) driving the 1793's DDEN, PC5 = drive select and PC6 = side. Still open: how PC5 is decoded into SEL0/SEL1 (MAME just switches drives), the 8" clock switch on PC3 (MAME has it as a TODO), and PA6, which MAME doesn't model.
- T3.2's clock (§10.2): RomBios 3.43m and the programs that use TIMCNT need a frame clock, RDEM.COM needs the schematic's 1.2308 MHz. MAME uses the frame clock; the emulator has both, the frame clock by default.
- The RAM disk's page latch (PA0–2 + PA5) and the way its card replaces the on-board RAM at 4000–BFFF in mode 1 are known from BIOS code only (§3.5). The factory's expander E6201 has no such latch (§10.5), so the card is unknown.
- The video circuit (§7.1–7.2) is read from the early board variant in the 1987 schematic and assumed unchanged on production boards. The clearer scan [EMM] confirms the address counter's clock (NAND of 1 MHz and HOR RTR) and the inputs of the LOAD gate.
- Whether the CPU ever waits for video: VIDEO CYCLE feeds the READY PROM D2 (§10.3), whose contents are unknown. A dump of D2 (ДГШ5.106.038, presumably), or a measurement of instruction timing on a real machine, would settle it.
- The mid-frame effects of §7.5 follow from the schematic and the 8253 data sheet but are untested. Unknown: how the МС6105 follows an irregular line rate or moved sync pulses, and whether picture shown during horizontal sync upsets the composite sync.

---

## 14. RomBios software interface [RB][ROM]

RomBios is the "resident network monitor", a derivative of the earlier JMON monitor. The manual describes v2.4/3.4; the supplied ROM is 3.43m. An emulator doesn't need any of this to run the ROM. It is here for HLE traps, debugging, and checking that a build behaves correctly.

### 14.1 Monitor commands (prompt `*`)
| Command | Function |
|---|---|
| `D xxxx/yyyy` | dump memory |
| `M xxxx/yyyy zzzz` | move |
| `C xxxx/yyyy zzzz` | compare |
| `K xxxx/yyyy bb` | search for differences from byte bb |
| `F xxxx/yyyy bb` | fill |
| `S xxxx` | modify memory (space keeps the byte, RETURN ends) |
| `G [xxxx][,yyyy]` | go, with an optional breakpoint |
| `E` / CTRL+C | echo key codes / back to command mode |
| `Pxx` | select external memory page xx (00–FDh) and open the window at 4000–BFFF |
| `PS` / `PR` | open / close the external window |
| `R xxxx/yyyy zzzz` | copy external memory to RAM |
| `W xxxx/yyyy zzzz` | copy RAM to external memory |
| `A` | load the program at the start of the selected external page to 0100h and run it |
| `B` | the same, from internal ROM |
| `T` | boot an operating system: `<D>isk, <T>ape, <N>et?`. Missing loaders are not offered; the supplied ROM shows only D and N. |

- **Bootable program header** (used by A, B and LDROMPRG): the image starts `JMP 0103h / DW key / DW length`. If key = CBAEh, the program starts automatically at power-on.
- **Typing commands** [ROM][EMU]: the monitor takes no spaces and completes fields itself. After the four hex digits of the first address it prints the `/`, and after the last field of a command it runs the command at once, without RETURN. So `D0000001F` is typed for `D 0000/001F`. A space, a typed `/` or a RETURN on an empty line is answered with `?`. Letters may be lower case. `G0100` followed by RETURN runs the program at 0100h.
- **Getting back to the monitor:** reset, or CALL FFC4h. CTRL+ESC jumps to 0000h; CTRL+SHIFT+ESC enters the monitor from the timer interrupt.

### 14.2 Entry points (jump table at FF26–FFFF, ROM 3F26–3FFF)
- **When they work.** They are callable in modes 1 and 2, where ROM is visible at D800–FFFF. MEMMD, PMEMMD and MEMMDRST also work in modes 0 and 3, because the BIOS keeps a RAM copy of the vector page there.
- **How dispatch works.** The `CALL D85Bh` entries dispatch to handler = (entry + 3 + 1702h). They switch to mode 0, run the handler in low ROM, then restore the mode.
- **Stack rules.** Keep SP at 8800h or above. In mode 0 keep it at 8000h or above, and outside 4000–BFFF while mode 2 is used. Below 8800h the dispatcher switches to an internal stack.

`*` = all registers preserved.

**Console and keyboard**

| Entry | Name | Function |
|---|---|---|
| FFC4 | MONITOR | monitor warm start |
| FF6E | MONVER | D = major + 30h, E = minor + 30h |
| FFD3 | TTI | read key, no echo → A |
| FFD6 | TTIO | read key with echo → A |
| FFC7 | TTSTAT* | Z = 0 and A = code if a key is waiting |
| FF98 | CONSTAT | A = FFh if a key is waiting, else 00h |
| FF71 | KEYBRD | scan the keyboard directly, for when the timer interrupt is off; call at most every 60 ms |
| FFD9 | TTO* | print character A (§14.4) |
| FFCA | CRLF | new line |
| FFCD | TTCON | print string at BC, ends at 00h or `$` |
| FFD0 | TTCLF | print string at BC, then a new line |
| FFDC / FFDF | OUTH* / OUTH2* | print A / BC in hex |
| FFE2 / FFE5 | INHEX* / INX2* | read hex into A / BC; CY = 1 on error |
| FFE8 | NIBBLE | CY = 0 if A is a hex digit |
| FF68 | PICTON | reset the 10-minute screen blanker; CY = 1 if the screen was off |

**Memory**

| Entry | Name | Function |
|---|---|---|
| FFF1 | DMEM | dump memory BC to HL |
| FFF4 | MOVE | copy BC..HL to DE |
| FFF7 | COMP | compare BC..HL with DE |
| FFFA | FILL | fill BC..HL with A |
| FFFD | CNST | search BC..HL for differences from A |
| FFB9 | MEMMD | select a temporary addressing mode A (0–3) |
| FFBC | PMEMMD | select the permanent addressing mode A. Power-on default is mode 1. |
| FFBF | MEMMDRST | restore the permanent mode |
| FF83 | MEMCOPY | copy HL bytes (a multiple of 128) from BC to DE; A = source mode + 16·destination mode |
| — | EXTPG | select / read the external page: A = 0–FDh select, FEh = none, FFh = read. The manual prints the entry as "FEBDH", which collides with PMEMMD at FFBCh. The supplied ROM has no such entry: its page code is internal and serves its RAM disk (ROM 2A93h, §3.5). |
| FF9B | LDROMPRG | load and run the program in ROM (A/B header) |

**Printer**

| Entry | Name | Function |
|---|---|---|
| FFEE | PRINTCHAR | print A. The driver vector is at the address held in FFEFh; the default is for СМ6329.02 on the user port. CTRL+SHIFT+P toggles echo to the printer. |
| FF77 | HARDCOPY | print the screen; also on ERASE, ESC, P or CTRL+SHIFT+F1 |

**Sound and timers**

| Entry | Name | Function |
|---|---|---|
| FF86 | BLEEP | A = duration / **100 ms** (the ROM sets a countdown of 5·A frame ticks, ROM 058Ah; the manual's "20 ms" is wrong for 3.43m); DE = 2 000 000 / frequency in Hz, or 0 = off. The tone plays at the current BLEEPVOL level, which is low after reset. [ROM][EMU] |
| FFB0 | BLEEPVOL | A = 0 for low volume, otherwise high. Key clicks use low volume. |
| FF9E | INTCNT | one-shot timer: DE = delay / 20 ms; DE = 0 reads and clears the flag, A = FFh when expired |
| FFA1 | TIMCNT | stopwatch: E = 0 resets, E = FFh reads into DE (in 20 ms units). The frame interrupt adds T3.2's count to it, so the units hold only with T3.2 on a frame clock (§10.2). [ROM] |
| FF89 | INTRSY | hook interrupt level A to handler DE (0 = release). Returns the previous handler in DE, which the new handler must chain to. |
| FF7A | (undocumented) | 8259 mask bit A: C = FFh masks, C = 0 unmasks |

**Graphics**

| Entry | Name | Function |
|---|---|---|
| FF8C | HGR | graphics mode: cursor to (0,0), colour white |
| FF8F | HCOLOR | E = 0 black, otherwise white |
| FF92 | HPLOT | plot point X = DE, Y = B |
| FF95 | HPLOTT | draw a line to X = DE, Y = B |

Coordinates are X 0–319, Y 0–239 in the 40×24 and 53×24 formats, and X 0–383, Y 0–199 in 64×20.

**Network** (8251 at 08h)

| Entry | Name |
|---|---|
| FF26 | SETPARM (DE = ROM parameter table; merges SWSTATE) |
| FF29 | GETPARM |
| FF2C | NETSTART |
| FF2F | ONGROUP |
| FF32 | OFFGROUP |
| FF35 | MWRITE |
| FF38 | MREADID |
| FF3B | MREAD |
| FF3E | MIGNORE |
| FF41 | STMOUT |
| FF44 | RTMOUT |

Network timers count in 20 ms units.

**Disk BIOS** (undocumented in this manual): FF4D, FF50, FF53 (FDC command), FF56, FF59, FF5C.

### 14.3 System variables
Several variables are reached through pointers stored in ROM, so their RAM addresses can change between versions. In the supplied ROM:

| Pointer | Name | RAM address | Content |
|---|---|---|---|
| (FF60h) | LATSYM | D7FBh → **ROM 1D02h** (runs at DD02h) | Latin/Estonian font, from code 20h |
| (FF62h) | RUSSYM | D7FDh → **ROM 2062h** (E062h) | Cyrillic font, 64 letters, codes C0–FFh |
| (FFC2h) | EXTSYM | D7F9h → 0000h | user font for codes 80–BFh; 0 = none |
| (FF64h) | USEREND | D7F7h | start of system area (for TSRs) |
| (FF66h) | LINELEN | D7FFh | characters per line: 40, 53 or 64 |
| (FFB7h) | NOWATY / NOWATX | D488h / D489h | graphics cursor Y / X |
| (FFA5h) | SQIKEY | D4BEh | code of the pressed key, 0 = none |
| −1 | SQKBSW | D4BDh | bit 0 = key pending |
| +2 | SQHCODER | D4C0h | bits 0–5: row (1–6); bit 6: SHIFT; bit 7: CTRL |
| +3 | SQHCODEC | D4C1h | column (1–16) |
| −3 | CONCH | D4BBh | keyboard/console control flags (cursor-position query, LOS/disk OS, CTRL+P echo; bit 7 = key click off, §14.6) |
| −4 | CONSW | D4BAh | bit 0 = CAPS LOCK, bit 1 = RUS. Start-up clears it (ROM 18C0h). The keyboard handler flips bit 0 on CAPS (code 81h) and bit 1 on LAT/RUS (82h) if a Cyrillic font is installed (RUSSYM high byte ≠ 0). The only feedback is the key click. |
| −5 | SWSTATE | D4B9h | keyboard DIP switches = network settings (§6.4) |
| −6 | FUNKEY | D4B8h | code of the function key pressed with SHIFT+CTRL |

The offsets in the first column are relative to the address stored at FFA5h, which is D4BEh.

**Font format:** 9 bytes per character, first byte = top row, **bit 7 = leftmost pixel**. Cells are 8×10, with the 10th row always blank. Capitals use a 7×5 field and lower case 5×5, with descenders and ascenders in the extra rows. The 53- and 64-column modes use 6×10 cells.

### 14.4 Display control codes (TTO)

**Single codes**

| Code | Action |
|---|---|
| 07 | bell |
| 08 | cursor left |
| 09 | tab (every 8 columns) |
| 0A, 0B, 0C | cursor down (scrolls) |
| 0D | carriage return |
| 1B | ESC prefix |

Other codes below 20h are ignored.

**ESC sequences** (an optional `[` after ESC is ignored)

| Sequence | Action |
|---|---|
| `ESC H` | home |
| `ESC L` | home and clear screen |
| `ESC = r v` or `ESC Y r v` | move cursor to row r, column v (each + 20h) |
| `ESC J` | clear to end of screen |
| `ESC K` | clear to end of line |
| `ESC D` / `C` / `B` / `A` | cursor left / right / down / up |
| `ESC R` | report cursor position; the next two key reads return r and v |
| `ESC 0` / `1` | key click off / on |
| `ESC 2` / `3` | scrolling off / on |
| `ESC 4` / `5` | cursor and output off / on |
| `ESC :` / `;` | line / page scroll mode |
| `ESC '` / `(` | reverse video on / off |
| `ESC M n` | text format: 0 = 40×24, 1 = 53×24, 2 = 64×20 |
| `ESC # r1 v1 r2 v2` | open a window (in 53- and 64-column modes, v1 and v2+1 must be multiples of 4) |
| `ESC N` | close the window |

`ESC M` and the window sequences need the extended-text monitor versions. The cursor blinks in LAT mode and is steady in RUS mode.

### 14.5 Character set (KOI-8 based, Table 3)
- **00–1Fh** (control codes produced by the keyboard):

| Code | Key | | Code | Key |
|---|---|---|---|---|
| 01 | F5 | | 0B | ↑ |
| 04 | F2 | | 0C | → |
| 05 | F3 | | 0D | RETURN |
| 06 | F6 | | 13 | F1 |
| 07 | BEL | | 17 | F7 |
| 08 | ← / BS | | 18 | F4 |
| 09 | TAB | | 1A | F8 |
| 0A | ↓ | | 1B | ESC |

- **20–7Fh:** ASCII, except that eight positions show Estonian letters instead:

| Code | Letter | | Code | Letter |
|---|---|---|---|---|
| 40h (@) | Ö | | 7Bh ({) | ö |
| 5Ch (\\) | Ä | | 7Ch (\|) | ä |
| 5Eh (^) | Ü | | 7Dh (}) | õ |
| 60h (`) | Õ | | 7Eh (~) | ü |

  7Fh is DEL.
- **80–BFh:** user-definable (EXTSYM).
- **C0–DFh:** lower-case Cyrillic and **E0–FFh:** upper-case Cyrillic, in KOI-8 order: ю а б ц д е ф г х и й к л м н о п я р с т у ж в ь ы з ш э щ ч ъ.
  - DFh (ъ) is typed as CTRL + "223".
  - FFh (RUS+DEL) displays as a space.
  - Any code 20–FFh can be typed by holding CTRL, entering the decimal code and releasing CTRL.

### 14.6 Key click [ROM][EMU]
- Every key that enters the buffer clicks. The keyboard handler (ROM 1253h) stores the code. Then, unless CONCH bit 7 is set, it runs the code at ROM 125Fh–1272h: it clears PA4 (quiet volume) and jumps to the BLEEP tail at 0597h with HL = 1 tick and DE = 03E8h (2 kHz). The BLEEP countdown runs on the frame interrupt, so the tone lasts 20–40 ms.
- `ESC 0` sets CONCH bit 7 (and BLEEPVOL high), and `ESC 1` clears it. Start-up writes 48h to CONCH (ROM 11F8h), so the click is on. Software can turn it back on at any time.
- **Emulator option.** Replacing the `JMP 0597h` at ROM 1270h with `RET` (C3h → C9h) removes the tone and keeps the volume change. The ROM test's sum for chip 2 (ROM 0008h, §11) goes up by 6, from 14h to 1Ah. RomBios 3.43 #0031 has the same code at the same address. In 3.43 #0035 it is 6 bytes later, at 1265h–1278h (JMP at 1276h), with the same chip sum at 0008h.
