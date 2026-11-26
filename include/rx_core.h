/*
 * Rusted Excel - native spreadsheet core.
 *
 * The engine is written in C++ and exported through a plain C ABI so the
 * Rust frontend can talk to it without any heavyweight bridges.
 */
#ifndef RX_CORE_H
#define RX_CORE_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct RxSheet RxSheet;

/* Sheet lifecycle ------------------------------------------------------- */
RxSheet *rx_sheet_new(int rows, int cols);
void rx_sheet_free(RxSheet *sheet);
int rx_sheet_rows(const RxSheet *sheet);
int rx_sheet_cols(const RxSheet *sheet);
void rx_sheet_resize(RxSheet *sheet, int rows, int cols);
void rx_sheet_clear(RxSheet *sheet);

/* Cells ----------------------------------------------------------------- */
/* raw: exactly what the user typed.  display: evaluated/formatted value.
 * Returned pointers are owned by the sheet and stay valid until the next
 * mutating call.  Copy them if you need to keep them. */
void rx_sheet_set(RxSheet *sheet, int row, int col, const char *text);
const char *rx_sheet_raw(const RxSheet *sheet, int row, int col);
const char *rx_sheet_display(RxSheet *sheet, int row, int col);
double rx_sheet_number(RxSheet *sheet, int row, int col);
int rx_sheet_is_formula(const RxSheet *sheet, int row, int col);

/* A1 helpers ------------------------------------------------------------ */
/* Writes e.g. "AB12" into buf. Returns bytes written (excluding NUL). */
size_t rx_cell_name(int row, int col, char *buf, size_t buf_len);
/* Parses "AB12" into row/col (0-based). Returns 1 on success. */
int rx_cell_parse(const char *name, int *row, int *col);

/* Lightweight CSV reader/writer (implemented in C) ---------------------- */
typedef void (*rx_csv_cell_fn)(void *user, int row, int col,
                               const char *text, size_t len);

/* Parses UTF-8 CSV. Calls `cb` for every field, including empty ones.
 * Returns 0 on success, -1 on malformed input. */
int rx_csv_parse(const char *input, rx_csv_cell_fn cb, void *user);

/* Escapes one field into `out`; returns the number of bytes needed including
 * the terminating NUL. Call with out == NULL to query the size. */
size_t rx_csv_escape(const char *field, char *out, size_t out_len);

#ifdef __cplusplus
}
#endif

#endif /* RX_CORE_H */
