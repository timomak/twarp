#pragma once

#include <stdbool.h>
#include <stdint.h>

typedef struct {
    uint8_t r;
    uint8_t g;
    uint8_t b;
    uint8_t a;
} TwarpComputerControlColor;

typedef struct {
    bool screen_recording_preflight_granted;
    bool screen_recording_granted;
    bool screen_recording_probe_supported;
    bool accessibility_preflight_granted;
    bool accessibility_granted;
    bool accessibility_probe_supported;
} TwarpComputerControlPermissionSnapshot;

typedef enum {
    TwarpComputerControlPermissionGranted = 0,
    TwarpComputerControlPermissionMissing = 1,
    TwarpComputerControlPermissionRestartRequired = 2,
    TwarpComputerControlPermissionDeniedOrUnknown = 3,
} TwarpComputerControlPermissionState;

typedef void (*TwarpComputerControlStopCallback)(void *context);
typedef void (*TwarpComputerControlPermissionCallback)(void *context);

TwarpComputerControlPermissionSnapshot twarp_computer_control_permissions_preflight(bool prompt_missing);

void *twarp_computer_control_permissions_panel_create(
    const char *session_label,
    TwarpComputerControlPermissionState screen_recording_state,
    TwarpComputerControlPermissionState accessibility_state,
    TwarpComputerControlColor panel_color,
    TwarpComputerControlColor text_color,
    TwarpComputerControlColor muted_text_color,
    TwarpComputerControlColor accent_color,
    TwarpComputerControlPermissionCallback retry_callback,
    void *retry_context,
    TwarpComputerControlPermissionCallback dismiss_callback,
    void *dismiss_context);

void twarp_computer_control_permissions_panel_update(
    void *host,
    const char *session_label,
    TwarpComputerControlPermissionState screen_recording_state,
    TwarpComputerControlPermissionState accessibility_state,
    TwarpComputerControlColor panel_color,
    TwarpComputerControlColor text_color,
    TwarpComputerControlColor muted_text_color,
    TwarpComputerControlColor accent_color);

void twarp_computer_control_permissions_panel_close(void *host);

void *twarp_computer_control_overlay_create(
    const char *session_label,
    const char *status_label,
    const char *action_log,
    bool confirmation_pending,
    TwarpComputerControlColor panel_color,
    TwarpComputerControlColor text_color,
    TwarpComputerControlColor muted_text_color,
    TwarpComputerControlColor glow_color,
    TwarpComputerControlStopCallback stop_callback,
    void *stop_context,
    TwarpComputerControlStopCallback approve_callback,
    void *approve_context,
    TwarpComputerControlStopCallback reject_callback,
    void *reject_context);

void twarp_computer_control_overlay_update(
    void *host,
    const char *session_label,
    const char *status_label,
    const char *action_log,
    bool confirmation_pending,
    TwarpComputerControlColor panel_color,
    TwarpComputerControlColor text_color,
    TwarpComputerControlColor muted_text_color,
    TwarpComputerControlColor glow_color);

void twarp_computer_control_overlay_close(void *host);
