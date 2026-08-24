#ifndef MADOPILOT_OCR_PRIVATE_FIXTURE_H
#define MADOPILOT_OCR_PRIVATE_FIXTURE_H

#include "madopilot/madopilot.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Explicitly non-production helper exported only by a library built with the
 * `private-fixture` Cargo feature. It wires fixed local OCR output into a replay
 * engine; all subsequent operations use the ordinary negotiated ABI 1.3 table. */
MADOPILOT_EXPORT madopilot_status_t madopilot_fixture_engine_create(
    const madopilot_source_t* source,
    const madopilot_engine_options_t* options,
    const madopilot_operation_t* operation,
    madopilot_engine_t** out_engine,
    madopilot_error_t** out_error);
#define MADOPILOT_FIXTURE_OCR_MODEL_ID "fixture-ocr-model"
#define MADOPILOT_FIXTURE_OCR_BACKEND_ID "fixture-ocr-backend"
#define MADOPILOT_FIXTURE_OCR_BACKEND_VERSION "1"

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* MADOPILOT_OCR_PRIVATE_FIXTURE_H */
