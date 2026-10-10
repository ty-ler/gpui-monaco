/* ASCII <ctype.h> functions that some grammar scanners call but tree-sitter's
 * wasm libc doesn't provide (see cc). */
#ifndef GPUI_MONACO_WASM_CTYPE_EXTRA_H_
#define GPUI_MONACO_WASM_CTYPE_EXTRA_H_

#include <ctype.h>

static inline int isdigit(int c) { return c >= '0' && c <= '9'; }
static inline int islower(int c) { return c >= 'a' && c <= 'z'; }
static inline int isupper(int c) { return c >= 'A' && c <= 'Z'; }
static inline int isalpha(int c) { return islower(c) || isupper(c); }
static inline int isalnum(int c) { return isalpha(c) || isdigit(c); }
static inline int isxdigit(int c) {
  return isdigit(c) || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F');
}
static inline int isspace(int c) { return c == ' ' || (c >= '\t' && c <= '\r'); }
static inline int ispunct(int c) { return isprint(c) && !isalnum(c) && c != ' '; }
static inline int tolower(int c) { return isupper(c) ? c + 32 : c; }
static inline int toupper(int c) { return islower(c) ? c - 32 : c; }

#endif
