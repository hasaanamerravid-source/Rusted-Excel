/* Rusted Excel - tiny RFC-4180-ish CSV reader/writer helper.
 * Deliberately plain C so the Rust side only needs a C ABI. */

#include "rx_core.h"

#include <stdlib.h>
#include <string.h>

static int rx_is_delimiter(char ch) {
    return ch == ',' || ch == '\n' || ch == '\r' || ch == '\0';
}

int rx_csv_parse(const char *input, rx_csv_cell_fn cb, void *user) {
    if (!input) return -1;

    const size_t n = strlen(input);
    char *buf = (char *)malloc(n + 2);
    if (!buf) return -1;

    int row = 0;
    int col = 0;
    size_t len = 0;
    int in_quotes = 0;
    int at_field_start = 1;
    int after_quote = 0;
    /* A field exists after a comma; a final newline does not start a new row. */
    int pending_field = 1;

    for (size_t i = 0; i <= n; ++i) {
        const char ch = (i < n) ? input[i] : '\0';

        if (in_quotes) {
            if (after_quote) {
                after_quote = 0;
                if (ch == '"') {
                    buf[len++] = '"'; /* escaped quote */
                    continue;
                }
                if (!rx_is_delimiter(ch)) {
                    free(buf);
                    return -1; /* e.g. "a"b is malformed */
                }
                in_quotes = 0; /* fall through: delimiter or EOF */
            } else {
                if (ch == '"') {
                    after_quote = 1;
                    continue;
                }
                if (ch == '\0') {
                    free(buf);
                    return -1; /* unterminated quoted field */
                }
                buf[len++] = ch;
                continue;
            }
        }

        if (at_field_start && ch == '"' && i < n) {
            in_quotes = 1;
            after_quote = 0;
            at_field_start = 0;
            continue;
        }

        if (ch == '\0') {
            if (pending_field && cb) cb(user, row, col, buf, len);
            break;
        }

        if (rx_is_delimiter(ch)) {
            if (cb) cb(user, row, col, buf, len);
            len = 0;
            at_field_start = 1;
            if (ch == ',') {
                ++col;
                pending_field = 1; /* empty final field is valid */
            } else { /* \n or \r */
                ++row;
                col = 0;
                pending_field = 0; /* a trailing newline is not a new row */
                if (ch == '\r' && i + 1 < n && input[i + 1] == '\n') ++i;
            }
            continue;
        }

        if (ch == '"') { /* bare quote inside an unquoted field */
            free(buf);
            return -1;
        }
        buf[len++] = ch;
        at_field_start = 0;
    }

    free(buf);
    return 0;
}

size_t rx_csv_escape(const char *field, char *out, size_t out_len) {
    if (!field) field = "";
    const int quoted = strpbrk(field, ",\"\r\n") != NULL;
    size_t needed = 1; /* terminating NUL */
    if (quoted) needed += 2;
    for (const char *p = field; *p; ++p) {
        needed += (*p == '"') ? 2 : 1;
    }

    if (!out || out_len == 0) return needed;

    size_t i = 0;
#define RX_PUT(c)                                                              \
    do {                                                                       \
        if (i + 1 < out_len) out[i++] = (c);                                    \
    } while (0)

    if (quoted) RX_PUT('"');
    for (const char *p = field; *p; ++p) {
        if (*p == '"') RX_PUT('"');
        RX_PUT(*p);
    }
    if (quoted) RX_PUT('"');
#undef RX_PUT

    out[i < out_len ? i : out_len - 1] = '\0';
    return needed;
}
