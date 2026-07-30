#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "computer_control_overlay.h"

typedef void (*TwarpComputerControlExtrasCallback)(void *context);

// ---------------------------------------------------------------------------
// Fake cursor overlay: a capture-excluded arrow that glides between the
// points computer control acts on. Safe to call from any thread.
// ---------------------------------------------------------------------------

void twarp_computer_control_cursor_show(void);

// `x`/`y` are physical screen pixels with a top-left origin (the coordinate
// space computer_use injects events in).
void twarp_computer_control_cursor_move(double x, double y, bool animate);

void twarp_computer_control_cursor_hide(void);

// ---------------------------------------------------------------------------
// Menu-bar status item shown while control is live. The single menu entry
// ("Stop Using <target>" / "Stop Computer Control") fires `callback` once.
// Safe to call from any thread.
// ---------------------------------------------------------------------------

void twarp_computer_control_status_item_show(
    const char *stop_title,
    TwarpComputerControlExtrasCallback callback,
    void *context);

void twarp_computer_control_status_item_set_title(const char *stop_title);

void twarp_computer_control_status_item_hide(void);

// ---------------------------------------------------------------------------
// App targeting. Safe to call from any thread.
// ---------------------------------------------------------------------------

typedef struct {
    int32_t pid;
    char name[256];
    char bundle_id[256];
} TwarpComputerControlAppInfo;

// Resolves a running app by bundle id or (partial, case-insensitive) name.
bool twarp_computer_control_resolve_app(const char *query, TwarpComputerControlAppInfo *out);

// Bounds of the app's focused (or first) window in physical screen pixels,
// top-left origin. Requires Accessibility.
bool twarp_computer_control_app_window_bounds(
    int32_t pid,
    double *out_x,
    double *out_y,
    double *out_width,
    double *out_height);

// Brings the app frontmost so injected HID events reach it.
bool twarp_computer_control_activate_app(int32_t pid);

// ---------------------------------------------------------------------------
// Controlled-app badge: a small pill pinned to the target window's top-left
// (over the traffic lights). Clicking it fires `callback` (stop). Safe to
// call from any thread; showing again replaces the previous badge.
// ---------------------------------------------------------------------------

void twarp_computer_control_badge_show(
    int32_t pid,
    const char *label,
    TwarpComputerControlColor accent_color,
    TwarpComputerControlExtrasCallback callback,
    void *context);

void twarp_computer_control_badge_hide(void);
