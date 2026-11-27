#pragma once

// Rust emulation core (core/src/ffi.rs) and debug server (debugger/dap/src/ffi.rs).
// Keep in sync with those two and video.rs.

#import <stdbool.h>
#import <stddef.h>
#import <stdint.h>

typedef struct JukuMachine JukuMachine;
typedef struct JukuAudio JukuAudio;      // audio ring, shared with the audio thread

enum {
    JukuRasterBytes = 64,       // µs per raster line, from the start of hsync
    JukuRasterLines = 313,      // lines from the start of vsync
};

// Mirrors `video::Frame`: the raster the beam drew, 1 bpp, bit 7 = leftmost
// pixel. Byte x of line y was shown x µs after the start of horizontal sync,
// on the yth line after the start of vertical sync.
typedef struct {
    uint64_t seq;               // increments once per completed frame
    uint16_t active_x;          // where the timer counts put the picture:
    uint16_t active_y;          //   first byte and line in the raster,
    uint16_t active_w;          //   bytes per line
    uint16_t active_h;          //   and lines
    uint8_t  _pad[8];
    uint8_t  data[JukuRasterLines * JukuRasterBytes];
} JukuFrame;

_Static_assert(offsetof(JukuFrame, data) == 24, "JukuFrame layout must match video::Frame");
_Static_assert(sizeof(JukuFrame) == 24 + JukuRasterLines * JukuRasterBytes, "JukuFrame layout must match video::Frame");

enum {
    JukuCpuHz = 2000000,
    JukuKeyShift = 0x80,
    JukuKeyCtrl = 0x81,
    JukuMouseRight = 0x01,
    JukuMouseLeft = 0x02,
};

extern const JukuAudio *juku_audio_new(uint32_t sample_rate);
extern void             juku_audio_free(const JukuAudio *a);
extern size_t           juku_audio_pull(const JukuAudio *a, float *left, float *right, size_t frames);   // audio thread
extern size_t           juku_audio_fill(const JukuAudio *a);
extern void             juku_audio_mute(JukuMachine *m, bool muted);   // warp speed: stop queueing samples
extern JukuMachine     *juku_new(const uint8_t *rom, size_t rom_len, const JukuAudio *audio);
extern void             juku_free(JukuMachine *m);
extern void             juku_reset(JukuMachine *m, bool cold);
extern int32_t          juku_expansion_check(const uint8_t *data, size_t len);   // 0, −1 (size), −2 (no program header)
extern int32_t          juku_expansion_insert(JukuMachine *m, const uint8_t *data, size_t len);   // cold-starts; as check
extern void             juku_expansion_eject(JukuMachine *m);   // cold-starts
extern bool             juku_load_rom(JukuMachine *m, const uint8_t *rom, size_t rom_len);   // cold-starts; false if wrong size
extern void             juku_system_rom(const JukuMachine *m, uint8_t *out);   // 16 384 bytes, as the ROM's file has them
extern const uint8_t   *juku_expansion_rom(const JukuMachine *m, size_t *len);  // the window without its trailing FFh
extern uint64_t         juku_run_cycles(JukuMachine *m, uint64_t cycles);
extern uint64_t         juku_cycles(const JukuMachine *m);
extern const JukuFrame *juku_frame(const JukuMachine *m);
extern void             juku_key(JukuMachine *m, uint8_t key, bool down);   // key = column << 3 | row
extern void             juku_keys_release_all(JukuMachine *m);
extern void             juku_caps_lock(JukuMachine *m, bool on);   // guest Caps Lock follows this
extern bool             juku_key_down(const JukuMachine *m, uint8_t key);
extern uint8_t          juku_consw(const JukuMachine *m);   // RomBios: bit 0 CAPS LOCK, bit 1 RUS

// The keyboard as it looks (core/src/keyboard/layout.rs): positions and sizes
// in quarter keys, x from ESC's left edge, rows from the F row.
enum {
    JukuLegendPair,             // top at the top left, bottom at the bottom right
    JukuLegendFunction,         // F1, centered
    JukuLegendWord,             // small, centered: ESC, CTRL, ERASE, DEL
    JukuLegendWideWord,         // large, centered: TAB, RETURN, SHIFT
    JukuLegendTwoLines,         // CAPS LOCK, LAT RUS
    JukuLegendArrow,            // top is ← → ↑ or ↓
    JukuLegendBlank,            // the space bar
    JukuKeyboardLeft = -2,      // TAB's left edge
    JukuKeyboardRight = 63,     // RETURN's right edge
    JukuKeyboardRows = 6,
    JukuBadgeX = 38,            // the JUKU E5101 badge beside F8
    JukuBadgeWidth = 17,
};

typedef struct {
    uint8_t id;                 // column << 3 | row, JukuKeyShift or JukuKeyCtrl
    int8_t  x;
    uint8_t row;
    uint8_t width;
    uint8_t legend;
    char    top[8];             // UTF-8
    char    bottom[8];
} JukuKeycap;

extern size_t           juku_keycap_count(void);
extern bool             juku_keycap(size_t index, JukuKeycap *out);

// The keyboard window's picture (core/src/keyboard/drawing.rs): drawing
// commands in drawing units, which KeyboardView.m replays. A picture call
// returns the length and writes only if it fits in `capacity`.
enum {
    JukuDrawBackground = 0x01, JukuDrawMove = 0x02, JukuDrawLine = 0x03, JukuDrawQuad = 0x04,
    JukuDrawCubic = 0x05, JukuDrawClose = 0x06, JukuDrawFill = 0x10, JukuDrawLinear = 0x11,
    JukuDrawRadial = 0x12, JukuDrawStroke = 0x13, JukuDrawLabel = 0x20,
};

extern void             juku_keyboard_size(float *width, float *height);
extern size_t           juku_keyboard_picture(const bool *down, const bool *labeled, int32_t hover, uint8_t *out, size_t capacity);
extern size_t           juku_keycap_picture(size_t index, uint8_t *out, size_t capacity, float *width, float *height);
extern int32_t          juku_keycap_at(float x, float y);   // −1 for none
extern bool             juku_set_key_click(JukuMachine *m, bool on);   // patches the ROM; false if unknown ROM
extern void             juku_set_t32_fast_clock(JukuMachine *m, bool on);   // T3.2 at 1.23 MHz, not once a frame
extern void             juku_set_drive_sounds(JukuMachine *m, bool on);   // motor, stepping, disks in and out
extern void             juku_set_drive_sounds_volume(JukuMachine *m, float volume);   // 0–2, 1 by default
extern void             juku_set_drive_sounds_spread(JukuMachine *m, float spread);   // 0–1, 0.25 by default
extern void             juku_set_drive_idle_mute(JukuMachine *m, bool on);   // motor fades out 10 s after the last command
extern void             juku_set_dip(JukuMachine *m, uint8_t bank, uint8_t value);
extern void             juku_mouse(JukuMachine *m, int32_t dx, int32_t dy, uint8_t buttons);   // +y down; bit 1 left, bit 0 right
extern bool             juku_mouse_enabled(const JukuMachine *m);    // software has IR6 unmasked

// Disks: drive 0 = A:, 1 = B:. Check returns 0 or −1 (unsupported size); insert
// returns those or −2 (bad drive).
extern int32_t          juku_disk_check(size_t len);
extern int32_t          juku_disk_insert(JukuMachine *m, uint8_t drive, const uint8_t *data, size_t len, bool write_protect);
extern int32_t          juku_disk_insert_blank(JukuMachine *m, uint8_t drive);   // formatted and empty
extern void             juku_disk_eject(JukuMachine *m, uint8_t drive);
extern bool             juku_disk_dirty(const JukuMachine *m, uint8_t drive);
extern const uint8_t   *juku_disk_data(const JukuMachine *m, uint8_t drive, size_t *len);
extern void             juku_disk_clear_dirty(JukuMachine *m, uint8_t drive);
extern void             juku_disk_set_write_protect(JukuMachine *m, uint8_t drive, bool on);
extern bool             juku_disk_write_protected(const JukuMachine *m, uint8_t drive);
extern uint8_t          juku_drive_activity(const JukuMachine *m);   // bit per drive: command running

// Snapshots (spec §4.6): a file of sections holding the machine. Host
// sections have 4-byte tags starting with a lowercase letter. Errors: −1 not
// a snapshot, −2 damaged, −3 made by a newer version, −4 by an older one.
typedef struct {
    uint8_t        tag[4];
    const uint8_t *data;
    size_t         len;
} JukuSection;

// Thumbnail NULL: the core draws the screen. Returns NULL for a bad PNG or tag.
extern uint8_t         *juku_snapshot_save(const JukuMachine *m, const uint8_t *thumbnail, size_t thumbnail_len,
                                           const JukuSection *sections, size_t count, size_t *len);
extern void             juku_snapshot_free(uint8_t *data, size_t len);
extern int32_t          juku_snapshot_load(JukuMachine *m, const uint8_t *data, size_t len);   // nothing changes on error
// 0 with *out NULL if the file has no such section; *out points into data.
extern int32_t          juku_snapshot_section(const uint8_t *data, size_t len, const uint8_t tag[4],
                                              const uint8_t **out, size_t *out_len);

// Debug server (spec §12): DAP on 127.0.0.1, with non-blocking sockets and no
// threads. Poll it on every tick, after running the machine. Poll and
// set_paused return the state bits.
typedef struct JukuDap JukuDap;

enum {
    JukuDapConnected = 1,       // a client is connected
    JukuDapStopped = 2,         // the debugger has stopped the machine
};

extern JukuDap         *juku_dap_new(uint16_t port);   // NULL if the port is taken
extern void             juku_dap_free(JukuDap *d, JukuMachine *m);   // the machine runs on
extern uint32_t         juku_dap_poll(JukuDap *d, JukuMachine *m);
extern uint32_t         juku_dap_run(JukuDap *d, JukuMachine *m, uint64_t cycles, uint64_t *ran);   // juku_run_cycles, then poll
extern uint32_t         juku_dap_set_paused(JukuDap *d, JukuMachine *m, bool paused);   // the app's Pause, with a client
// A snapshot a launch asks the app to open: its UTF-8 path, its length, 0 for
// none. The app opens it as the user would and says whether it did.
extern size_t           juku_dap_snapshot_request(JukuDap *d, uint8_t *buf, size_t cap);
extern uint32_t         juku_dap_snapshot_opened(JukuDap *d, JukuMachine *m, bool opened);
