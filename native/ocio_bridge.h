#pragma once
#include <stddef.h>
#include <stdint.h>

extern "C" {
const char *antitone_ocio_version();
int antitone_output_parameters(uint32_t target, double *parameters);
void *antitone_ocio_create(uint32_t target, char *error, size_t error_size);
int antitone_ocio_apply(void *handle, float *rgb, size_t count, char *error, size_t error_size);
void antitone_ocio_destroy(void *handle);
}
