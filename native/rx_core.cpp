// Rusted Excel - a small, dependency-free spreadsheet engine.
// Exposes a C ABI (see include/rx_core.h) to the Rust GUI layer.

#include "rx_core.h"

#include <algorithm>
#include <cctype>
#include <climits>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

namespace {

struct Value {
    enum class Kind { Empty, Number, Text, Ref, Error };

    Kind kind = Kind::Empty;
    double number = 0.0;
    std::string text;
    int row = 0;
    int col = 0;

    static Value empty() { return Value(); }
    static Value num(double n) {
        Value v;
        v.kind = Kind::Number;
        v.number = n;
        return v;
    }
    static Value str(const std::string &s) {
        Value v;
        v.kind = Kind::Text;
        v.text = s;
        return v;
    }
    static Value ref(int r, int c) {
        Value v;
        v.kind = Kind::Ref;
        v.row = r;
        v.col = c;
        return v;
    }
    static Value err(const char *e) {
        Value v;
        v.kind = Kind::Error;
        v.text = e;
        return v;
    }
};

std::string format_number(double n) {
    if (!std::isfinite(n)) {
        return n > 0 ? "#NUM!" : "#NUM!";
    }
    if (n == 0.0) {
        return "0";
    }
    char buf[64];
    std::snprintf(buf, sizeof(buf), "%.10g", n);
    return std::string(buf);
}

struct Storage {
    std::vector<std::string> cells;
    std::vector<Value> values;
    std::vector<unsigned char> state; // 0 = new, 1 = evaluating, 2 = done
    std::vector<std::string> display;
    std::vector<unsigned char> display_valid;
};

Storage make_storage(int rows, int cols) {
    const size_t n = static_cast<size_t>(rows) * static_cast<size_t>(cols);
    Storage storage;
    storage.cells.assign(n, std::string());
    storage.values.assign(n, Value());
    storage.state.assign(n, 0);
    storage.display.assign(n, std::string());
    storage.display_valid.assign(n, 0);
    return storage;
}

struct Sheet {
    int rows;
    int cols;
    std::vector<std::string> cells;
    std::vector<Value> values;
    std::vector<unsigned char> state;
    std::vector<std::string> display;
    std::vector<unsigned char> display_valid;

    Sheet(int r, int c) : rows(r), cols(c) { reset(); }

    void install(Storage &fresh) {
        cells.swap(fresh.cells);
        values.swap(fresh.values);
        state.swap(fresh.state);
        display.swap(fresh.display);
        display_valid.swap(fresh.display_valid);
    }

    void reset() {
        Storage fresh = make_storage(rows, cols);
        install(fresh);
    }

    void resized(int new_rows, int new_cols) {
        Storage fresh = make_storage(new_rows, new_cols);
        const int copy_rows = std::min(rows, new_rows);
        const int copy_cols = std::min(cols, new_cols);
        for (int r = 0; r < copy_rows; ++r) {
            for (int c = 0; c < copy_cols; ++c) {
                fresh.cells[static_cast<size_t>(r) * new_cols + c] =
                    cells[static_cast<size_t>(r) * cols + c];
            }
        }
        rows = new_rows;
        cols = new_cols;
        install(fresh);
    }

    int index(int r, int c) const { return r * cols + c; }
    bool valid(int r, int c) const {
        return r >= 0 && c >= 0 && r < rows && c < cols;
    }

    void invalidate() {
        std::fill(state.begin(), state.end(), 0);
        std::fill(display_valid.begin(), display_valid.end(), 0);
    }
};

Value resolve_ref(const Value &v, Sheet &sheet, std::vector<unsigned char> &state);
Value eval_cell(Sheet &sheet, int row, int col, std::vector<unsigned char> &state);

// Recursive-descent formula parser. Only standard C/C++ runtime, no regex.
class Parser {
public:
    Parser(const std::string &source, Sheet &sheet,
           std::vector<unsigned char> &state)
        : src(source), sheet(sheet), state(state) {}

    Value parse_all() {
        Value v = parse_compare();
        if (is_error(v)) {
            return v;
        }
        skip();
        if (pos != src.size()) {
            return Value::err("#ERR!");
        }
        return v;
    }

private:
    std::string src;
    Sheet &sheet;
    std::vector<unsigned char> &state;
    size_t pos = 0;

    void skip() {
        while (pos < src.size() && std::isspace(static_cast<unsigned char>(src[pos]))) {
            ++pos;
        }
    }

    bool eat(char ch) {
        skip();
        if (pos < src.size() && src[pos] == ch) {
            ++pos;
            return true;
        }
        return false;
    }

    bool eat_op(const char *op) {
        skip();
        const size_t n = std::strlen(op);
        if (src.size() - pos >= n && src.compare(pos, n, op) == 0) {
            pos += n;
            return true;
        }
        return false;
    }

    bool eat_word(const char *word) {
        skip();
        size_t n = std::strlen(word);
        if (src.size() - pos < n) {
            return false;
        }
        for (size_t i = 0; i < n; ++i) {
            if (std::toupper(static_cast<unsigned char>(src[pos + i])) !=
                std::toupper(static_cast<unsigned char>(word[i]))) {
                return false;
            }
        }
        // Do not swallow a prefix of a longer identifier.
        if (pos + n < src.size() &&
            (std::isalnum(static_cast<unsigned char>(src[pos + n])) ||
             src[pos + n] == '_')) {
            return false;
        }
        pos += n;
        return true;
    }

    static bool is_error(const Value &v) { return v.kind == Value::Kind::Error; }

    Value resolve(const Value &v) { return resolve_ref(v, sheet, state); }

    bool has_error(const Value &v, Value &err) {
        if (is_error(v)) {
            err = v;
            return true;
        }
        Value x = resolve_ref(v, sheet, state);
        if (is_error(x)) {
            err = x;
            return true;
        }
        return false;
    }

    double as_number(const Value &v) {
        if (is_error(v)) {
            return 0.0;
        }
        Value x = resolve_ref(v, sheet, state);
        if (is_error(x)) {
            return 0.0;
        }
        if (x.kind == Value::Kind::Number) {
            return x.number;
        }
        if (x.kind == Value::Kind::Text) {
            char *end = nullptr;
            double d = std::strtod(x.text.c_str(), &end);
            if (end && *end == '\0') {
                return d;
            }
            return 0.0;
        }
        return 0.0;
    }

    std::string as_text(const Value &v) {
        Value x = resolve_ref(v, sheet, state);
        switch (x.kind) {
        case Value::Kind::Number:
            return format_number(x.number);
        case Value::Kind::Text:
            return x.text;
        case Value::Kind::Error:
            return x.text;
        case Value::Kind::Empty:
        case Value::Kind::Ref:
            return std::string();
        }
        return std::string();
    }

    Value parse_compare() {
        Value left = parse_concat();
        if (is_error(left)) {
            return left;
        }
        for (;;) {
            skip();
            const char *op = nullptr;
            if (eat_op("<>")) op = "<>";
            else if (eat_op("<=")) op = "<=";
            else if (eat_op(">=")) op = ">=";
            else if (eat('=')) op = "=";
            else if (eat('<')) op = "<";
            else if (eat('>')) op = ">";
            if (!op) {
                return left;
            }
            Value right = parse_concat();
            if (is_error(right)) {
                return right;
            }
            Value a = resolve_ref(left, sheet, state);
            Value b = resolve_ref(right, sheet, state);
            if (is_error(a)) return a;
            if (is_error(b)) return b;
            int cmp = 0;
            if (a.kind == Value::Kind::Text || b.kind == Value::Kind::Text) {
                std::string sa = as_text(a);
                std::string sb = as_text(b);
                std::transform(sa.begin(), sa.end(), sa.begin(),
                               [](unsigned char c) { return std::tolower(c); });
                std::transform(sb.begin(), sb.end(), sb.begin(),
                               [](unsigned char c) { return std::tolower(c); });
                cmp = sa < sb ? -1 : (sa > sb ? 1 : 0);
            } else {
                double da = as_number(a);
                double db = as_number(b);
                cmp = da < db ? -1 : (da > db ? 1 : 0);
            }
            bool result = false;
            if (std::strcmp(op, "=") == 0) result = cmp == 0;
            else if (std::strcmp(op, "<>") == 0) result = cmp != 0;
            else if (std::strcmp(op, "<") == 0) result = cmp < 0;
            else if (std::strcmp(op, ">") == 0) result = cmp > 0;
            else if (std::strcmp(op, "<=") == 0) result = cmp <= 0;
            else if (std::strcmp(op, ">=") == 0) result = cmp >= 0;
            left = Value::num(result ? 1.0 : 0.0);
        }
    }

    Value parse_concat() {
        Value left = parse_addsub();
        if (is_error(left)) {
            return left;
        }
        while (eat('&')) {
            Value right = parse_addsub();
            if (is_error(right)) {
                return right;
            }
            Value err;
            if (has_error(left, err) || has_error(right, err)) {
                return err;
            }
            left = Value::str(as_text(left) + as_text(right));
        }
        return left;
    }

    Value parse_addsub() {
        Value left = parse_muldiv();
        if (is_error(left)) {
            return left;
        }
        for (;;) {
            bool plus = eat('+');
            bool minus = !plus && eat('-');
            if (!plus && !minus) {
                return left;
            }
            Value right = parse_muldiv();
            if (is_error(right)) {
                return right;
            }
            Value err;
            if (has_error(left, err) || has_error(right, err)) {
                return err;
            }
            double a = as_number(left);
            double b = as_number(right);
            left = Value::num(plus ? a + b : a - b);
        }
    }

    Value parse_muldiv() {
        Value left = parse_unary();
        if (is_error(left)) {
            return left;
        }
        for (;;) {
            bool mul = eat('*');
            bool div = !mul && eat('/');
            if (!mul && !div) {
                return left;
            }
            Value right = parse_unary();
            if (is_error(right)) {
                return right;
            }
            Value err;
            if (has_error(left, err) || has_error(right, err)) {
                return err;
            }
            double b = as_number(right);
            if (div && b == 0.0) {
                return Value::err("#DIV/0!");
            }
            double a = as_number(left);
            left = Value::num(mul ? a * b : a / b);
        }
    }

    Value parse_power() {
        Value left = parse_primary();
        if (is_error(left)) {
            return left;
        }
        if (eat('^')) {
            Value right = parse_unary(); // right associative
            if (is_error(right)) {
                return right;
            }
            Value err;
            if (has_error(left, err) || has_error(right, err)) {
                return err;
            }
            return Value::num(std::pow(as_number(left), as_number(right)));
        }
        return left;
    }

    Value parse_unary() {
        if (eat('-')) {
            Value v = parse_unary();
            if (is_error(v)) return v;
            return Value::num(-as_number(v));
        }
        if (eat('+')) {
            Value v = parse_unary();
            if (is_error(v)) return v;
            return Value::num(as_number(v));
        }
        return parse_power();
    }

    // `A1:B3` is only meaningful as a function argument; parse_arg handles it.
    Value parse_primary() {
        skip();
        if (pos >= src.size()) {
            return Value::err("#ERR!");
        }
        char ch = src[pos];

        if (ch == '(') {
            ++pos;
            Value v = parse_compare();
            if (!eat(')')) {
                return Value::err("#ERR!");
            }
            return v;
        }
        if (ch == '"') {
            ++pos;
            std::string out;
            while (pos < src.size() && src[pos] != '"') {
                if (src[pos] == '\\' && pos + 1 < src.size()) {
                    ++pos;
                }
                out += src[pos++];
            }
            if (pos >= src.size()) {
                return Value::err("#ERR!");
            }
            ++pos;
            return Value::str(out);
        }
        if (ch == '$' || std::isalpha(static_cast<unsigned char>(ch)) || ch == '_') {
            size_t start = pos;
            if (ch == '$') ++pos;
            while (pos < src.size() &&
                   (std::isalnum(static_cast<unsigned char>(src[pos])) ||
                    src[pos] == '$' || src[pos] == '_')) {
                ++pos;
            }
            std::string word = src.substr(start, pos - start);
            skip();
            if (pos < src.size() && src[pos] == '(') {
                return parse_function(word);
            }

            std::string cleaned;
            for (char c : word) {
                if (c != '$') cleaned += c;
            }
            std::string upper;
            for (char c : cleaned) {
                upper += static_cast<char>(std::toupper(static_cast<unsigned char>(c)));
            }
            if (upper == "TRUE" || upper == "FALSE") {
                return Value::num(upper == "TRUE" ? 1.0 : 0.0);
            }

            std::string letters;
            std::string digits;
            bool seen_digit = false;
            bool valid = !cleaned.empty();
            for (char c : cleaned) {
                if (std::isdigit(static_cast<unsigned char>(c))) {
                    seen_digit = true;
                    digits += c;
                } else if (std::isalpha(static_cast<unsigned char>(c)) && !seen_digit) {
                    letters += static_cast<char>(std::toupper(static_cast<unsigned char>(c)));
                } else {
                    valid = false;
                }
            }
            if (valid && !letters.empty() && !digits.empty()) {
                long long col = 0;
                for (char c : letters) {
                    col = col * 26 + (c - 'A' + 1);
                    if (col > INT_MAX) return Value::err("#REF!");
                }
                long long row = 0;
                for (char c : digits) {
                    row = row * 10 + (c - '0');
                    if (row > INT_MAX) return Value::err("#REF!");
                }
                return Value::ref(static_cast<int>(row) - 1, static_cast<int>(col) - 1);
            }
            return Value::err("#NAME?");
        }
        if (std::isdigit(static_cast<unsigned char>(ch)) || ch == '.') {
            size_t start = pos;
            bool dot = false;
            while (pos < src.size() &&
                   (std::isdigit(static_cast<unsigned char>(src[pos])) ||
                    (src[pos] == '.' && !dot))) {
                if (src[pos] == '.') dot = true;
                ++pos;
            }
            return Value::num(std::strtod(src.substr(start, pos - start).c_str(), nullptr));
        }
        return Value::err("#ERR!");
    }

    Value parse_function(const std::string &raw_name) {
        if (!eat('(')) {
            return Value::err("#ERR!");
        }
        std::string name;
        for (char c : raw_name) {
            name += static_cast<char>(std::toupper(static_cast<unsigned char>(c)));
        }

        std::vector<Value> args;
        if (!eat(')')) {
            for (;;) {
                Value v = parse_compare();
                if (is_error(v)) {
                    return v;
                }
                if (v.kind == Value::Kind::Ref && eat(':')) {
                    Value other = parse_compare();
                    if (is_error(other)) return other;
                    if (other.kind != Value::Kind::Ref) {
                        return Value::err("#REF!");
                    }
                    int r1 = std::min(v.row, other.row);
                    int r2 = std::max(v.row, other.row);
                    int c1 = std::min(v.col, other.col);
                    int c2 = std::max(v.col, other.col);
                    if (!sheet.valid(r1, c1) || !sheet.valid(r2, c2)) {
                        return Value::err("#REF!");
                    }
                    for (int r = r1; r <= r2; ++r) {
                        for (int c = c1; c <= c2; ++c) {
                            args.push_back(eval_cell(sheet, r, c, state));
                        }
                    }
                } else {
                    args.push_back(v);
                }
                if (eat(')')) break;
                if (!eat(',')) return Value::err("#ERR!");
            }
        }

        if (name == "SUM" || name == "AVG" || name == "AVERAGE" ||
            name == "MIN" || name == "MAX" || name == "COUNT") {
            double acc = 0.0;
            int count = 0;
            double min_v = 0.0, max_v = 0.0;
            for (const Value &v : args) {
                Value x = resolve_ref(v, sheet, state);
                if (is_error(x)) return x;
                if (x.kind != Value::Kind::Number && x.kind != Value::Kind::Text &&
                    x.kind != Value::Kind::Empty) {
                    continue;
                }
                if (name == "COUNT") {
                    if (x.kind == Value::Kind::Number) ++count;
                    continue;
                }
                double n = as_number(x);
                if (count == 0) {
                    min_v = max_v = n;
                } else {
                    min_v = std::min(min_v, n);
                    max_v = std::max(max_v, n);
                }
                acc += n;
                ++count;
            }
            if (name == "COUNT") return Value::num(count);
            if (count == 0) return Value::num(0.0);
            if (name == "SUM") return Value::num(acc);
            if (name == "MIN") return Value::num(min_v);
            if (name == "MAX") return Value::num(max_v);
            return Value::num(acc / count);
        }
        if (name == "ABS" && args.size() == 1) {
            Value err;
            if (has_error(args[0], err)) return err;
            return Value::num(std::fabs(as_number(args[0])));
        }
        if (name == "SQRT" && args.size() == 1) {
            Value err;
            if (has_error(args[0], err)) return err;
            double n = as_number(args[0]);
            return n < 0 ? Value::err("#NUM!") : Value::num(std::sqrt(n));
        }
        if (name == "ROUND" && args.size() >= 1) {
            Value err;
            if (has_error(args[0], err)) return err;
            double places = args.size() > 1 ? as_number(args[1]) : 0.0;
            double factor = std::pow(10.0, places);
            return Value::num(std::round(as_number(args[0]) * factor) / factor);
        }
        if (name == "POW" && args.size() == 2) {
            Value err;
            if (has_error(args[0], err) || has_error(args[1], err)) return err;
            return Value::num(std::pow(as_number(args[0]), as_number(args[1])));
        }
        if (name == "IF" && args.size() >= 2) {
            Value err;
            if (has_error(args[0], err)) return err;
            bool cond = as_number(args[0]) != 0.0;
            Value result = cond ? args[1] : (args.size() > 2 ? args[2] : Value::str(""));
            return resolve_ref(result, sheet, state);
        }
        if (name == "LEN" && args.size() == 1) {
            Value err;
            if (has_error(args[0], err)) return err;
            const std::string text = as_text(args[0]);
            size_t chars = 0;
            for (unsigned char c : text) {
                if ((c & 0xC0) != 0x80) ++chars;
            }
            return Value::num(static_cast<double>(chars));
        }
        if (name == "UPPER" && args.size() == 1) {
            Value err;
            if (has_error(args[0], err)) return err;
            std::string s = as_text(args[0]);
            std::transform(s.begin(), s.end(), s.begin(),
                           [](unsigned char c) { return std::toupper(c); });
            return Value::str(s);
        }
        return Value::err("#NAME?");
    }
};

Value eval_cell(Sheet &sheet, int row, int col,
                std::vector<unsigned char> &state) {
    static thread_local int depth = 0;
    if (depth > 128) {
        return Value::err("#DEPTH!");
    }
    ++depth;
    struct DepthGuard {
        int &value;
        ~DepthGuard() { --value; }
    } guard{depth};
    if (!sheet.valid(row, col)) {
        return Value::err("#REF!");
    }
    const int idx = sheet.index(row, col);
    if (state[idx] == 1) {
        return Value::err("#CYCLE!");
    }
    if (state[idx] == 2) {
        return sheet.values[idx];
    }

    state[idx] = 1;
    const std::string &raw = sheet.cells[idx];
    Value result;
    if (raw.empty()) {
        result = Value::empty();
    } else if (raw[0] == '=') {
        Parser parser(raw.substr(1), sheet, state);
        result = parser.parse_all();
        // A formula consisting of just a cell reference still needs resolving.
        // Do it before caching so cycles are detected against the live state.
        if (result.kind == Value::Kind::Ref) {
            result = resolve_ref(result, sheet, state);
        }
    } else {
        char *end = nullptr;
        const char *begin = raw.c_str();
        double n = std::strtod(begin, &end);
        if (end != begin && *end == '\0') {
            result = Value::num(n);
        } else {
            result = Value::str(raw);
        }
    }
    sheet.values[idx] = result;
    state[idx] = 2;
    return result;
}

Value resolve_ref(const Value &v, Sheet &sheet, std::vector<unsigned char> &state) {
    if (v.kind == Value::Kind::Ref) {
        return eval_cell(sheet, v.row, v.col, state);
    }
    return v;
}

std::string value_to_text(const Value &v) {
    switch (v.kind) {
    case Value::Kind::Number: return format_number(v.number);
    case Value::Kind::Text:
    case Value::Kind::Error: return v.text;
    case Value::Kind::Ref: return "#REF!";
    case Value::Kind::Empty: return std::string();
    }
    return std::string();
}

Sheet *as_sheet(RxSheet *p) { return reinterpret_cast<Sheet *>(p); }
const Sheet *as_sheet(const RxSheet *p) { return reinterpret_cast<const Sheet *>(p); }

} // namespace

extern "C" {

RxSheet *rx_sheet_new(int rows, int cols) {
    rows = std::max(1, rows);
    cols = std::max(1, cols);
    try {
        return reinterpret_cast<RxSheet *>(new Sheet(rows, cols));
    } catch (...) {
        return nullptr;
    }
}

void rx_sheet_free(RxSheet *sheet) { delete as_sheet(sheet); }

int rx_sheet_rows(const RxSheet *sheet) {
    return sheet ? as_sheet(sheet)->rows : 0;
}

int rx_sheet_cols(const RxSheet *sheet) {
    return sheet ? as_sheet(sheet)->cols : 0;
}

void rx_sheet_resize(RxSheet *sheet, int rows, int cols) {
    if (!sheet) return;
    try {
        Sheet *s = as_sheet(sheet);
        rows = std::max(1, rows);
        cols = std::max(1, cols);
        if (rows == s->rows && cols == s->cols) return;
        s->resized(rows, cols);
    } catch (...) {
        // Keep the previous dimensions and contents on allocation failure.
    }
}

void rx_sheet_clear(RxSheet *sheet) {
    if (!sheet) return;
    try {
        as_sheet(sheet)->reset();
    } catch (...) {
    }
}

void rx_sheet_set(RxSheet *sheet, int row, int col, const char *text) {
    if (!sheet) return;
    try {
        Sheet *s = as_sheet(sheet);
        if (!s->valid(row, col)) return;
        s->cells[s->index(row, col)] = text ? text : "";
        s->invalidate();
    } catch (...) {
    }
}

const char *rx_sheet_raw(const RxSheet *sheet, int row, int col) {
    if (!sheet) return "";
    const Sheet *s = as_sheet(sheet);
    if (!s->valid(row, col)) return "";
    return s->cells[s->index(row, col)].c_str();
}

const char *rx_sheet_display(RxSheet *sheet, int row, int col) {
    if (!sheet) return "";
    try {
        Sheet *s = as_sheet(sheet);
        if (!s->valid(row, col)) return "";
        const int idx = s->index(row, col);
        if (!s->display_valid[idx]) {
            std::vector<unsigned char> state(s->cells.size(), 0);
            Value v = eval_cell(*s, row, col, state);
            std::string text = value_to_text(v);
            s->display[idx].swap(text);
            s->display_valid[idx] = 1;
        }
        return s->display[idx].c_str();
    } catch (...) {
        return "";
    }
}

double rx_sheet_number(RxSheet *sheet, int row, int col) {
    if (!sheet) return 0.0;
    try {
        Sheet *s = as_sheet(sheet);
        if (!s->valid(row, col)) return 0.0;
        std::vector<unsigned char> state(s->cells.size(), 0);
        Value v = eval_cell(*s, row, col, state);
        Value x = resolve_ref(v, *s, state);
        return x.kind == Value::Kind::Number ? x.number : 0.0;
    } catch (...) {
        return 0.0;
    }
}

int rx_sheet_is_formula(const RxSheet *sheet, int row, int col) {
    const char *raw = rx_sheet_raw(sheet, row, col);
    return raw && raw[0] == '=' ? 1 : 0;
}

size_t rx_cell_name(int row, int col, char *buf, size_t buf_len) {
    if (!buf || buf_len == 0 || row < 0 || col < 0 || row == INT_MAX || col == INT_MAX) {
        return 0;
    }
    char letters[16];
    size_t len = 0;
    long long n = static_cast<long long>(col) + 1;
    while (n > 0 && len < sizeof(letters)) {
        int rem = static_cast<int>((n - 1) % 26);
        letters[len++] = static_cast<char>('A' + rem);
        n = (n - 1) / 26;
    }
    std::reverse(letters, letters + len);
    char digits[32];
    int dn = std::snprintf(digits, sizeof(digits), "%lld", static_cast<long long>(row) + 1);
    if (dn < 0) dn = 0;

    size_t total = 0;
    for (size_t i = 0; i < len && total + 1 < buf_len; ++i) buf[total++] = letters[i];
    for (int i = 0; i < dn && total + 1 < buf_len; ++i) buf[total++] = digits[i];
    buf[total] = '\0';
    return total;
}

int rx_cell_parse(const char *name, int *row, int *col) {
    if (!name) return 0;
    long long c = 0;
    size_t i = 0;
    while (name[i] && std::isalpha(static_cast<unsigned char>(name[i]))) {
        c = c * 26 + (std::toupper(static_cast<unsigned char>(name[i])) - 'A' + 1);
        if (c > INT_MAX) return 0;
        ++i;
    }
    if (i == 0 || !std::isdigit(static_cast<unsigned char>(name[i]))) return 0;
    long long r = 0;
    while (name[i]) {
        if (!std::isdigit(static_cast<unsigned char>(name[i]))) return 0;
        r = r * 10 + (name[i] - '0');
        if (r > INT_MAX) return 0;
        ++i;
    }
    if (r < 1 || c < 1) return 0;
    if (row) *row = static_cast<int>(r) - 1;
    if (col) *col = static_cast<int>(c) - 1;
    return 1;
}

} // extern "C"
