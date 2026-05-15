// Minimal `<esp_timer.h>` shim — the seven `kernels/esp_nn/*.cc` op
// implementations unconditionally `#include <esp_timer.h>` and call
// `esp_timer_get_time()` to accumulate a per-op profiling counter
// (`add_total_time`, `conv_total_time`, …) inside the kernels. The
// counters aren't read anywhere in TFLM library code — they exist so
// downstream ESP-IDF apps can poll them for inference-time benchmarks.
//
// We don't link against ESP-IDF (the firmware is bare-metal embassy
// + esp-hal), so we provide our own header that satisfies the
// declaration and an inline body that returns 0. The kernels then
// compile cleanly; the unused counter increments become dead writes
// the optimiser drops.
//
// If a future task needs real timing, swap the inline body for a
// weak symbol the firmware can override with `embassy_time::Instant`.

#ifndef ESP_TFLITE_MICRO_SYS_ESP_TIMER_H_
#define ESP_TFLITE_MICRO_SYS_ESP_TIMER_H_

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

static inline int64_t esp_timer_get_time(void) { return 0; }

#ifdef __cplusplus
}
#endif

#endif  // ESP_TFLITE_MICRO_SYS_ESP_TIMER_H_
