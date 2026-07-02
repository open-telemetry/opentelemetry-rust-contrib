/*
 * opentelemetry_c/api.h
 *
 * Umbrella header for the OpenTelemetry C API. Include this single header to pull in
 * the full traces API surface:
 *
 *   #include <opentelemetry_c/api.h>
 *
 * Or include the individual headers directly:
 *
 *   #include <opentelemetry_c/common.h>   // status, string views, attributes, version
 *   #include <opentelemetry_c/trace.h>    // tracer provider, tracer, span
 *   #include <opentelemetry_c/sdk.h>      // SDK builder + lifecycle
 *
 * Status: EXPERIMENTAL. The ABI is not yet stable (see README.md). Metrics and logs
 * are intentionally not part of this slice and will be added in separate headers
 * without breaking the traces ABI.
 */
#ifndef OPENTELEMETRY_C_API_H
#define OPENTELEMETRY_C_API_H

#include "common.h"
#include "sdk.h"
#include "trace.h"

#endif /* OPENTELEMETRY_C_API_H */
