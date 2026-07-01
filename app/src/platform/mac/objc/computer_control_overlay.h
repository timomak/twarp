#pragma once

#include <stdint.h>

typedef struct {
    uint8_t r;
    uint8_t g;
    uint8_t b;
    uint8_t a;
} TwarpComputerControlColor;

typedef void (*TwarpComputerControlStopCallback)(void *context);

void *twarp_computer_control_overlay_create(
    const char *session_label,
    TwarpComputerControlColor panel_color,
    TwarpComputerControlColor text_color,
    TwarpComputerControlColor muted_text_color,
    TwarpComputerControlColor glow_color,
    TwarpComputerControlStopCallback stop_callback,
    void *stop_context);

void twarp_computer_control_overlay_update(
    void *host,
    const char *session_label,
    TwarpComputerControlColor panel_color,
    TwarpComputerControlColor text_color,
    TwarpComputerControlColor muted_text_color,
    TwarpComputerControlColor glow_color);

void twarp_computer_control_overlay_close(void *host);
