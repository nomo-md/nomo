#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

char *nomo_preview_create(const uint8_t *bytes, size_t length, bool dark,
                         bool (*cancelled)(void *), void *context);
void nomo_preview_free(char *value);
