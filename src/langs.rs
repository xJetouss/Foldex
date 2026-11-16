//! Sprachwissen für die Vorschläge im Editor: Schlüsselwörter, Standardfunktionen und -typen sowie häufige
//! Methoden (nach „.“) je Sprache, dazu die Kommentar- und Zeichenkettenregeln für die Bezeichner-Suche.

pub struct Lang {
    pub name: &'static str,
    pub keywords: &'static [&'static str],
    /// Standardfunktionen, Makros, eingebaute Typen
    pub builtins: &'static [&'static str],
    /// häufige Methoden/Eigenschaften nach „.“
    pub members: &'static [&'static str],
    /// Zeilenkommentar(e)
    pub line_comment: &'static [&'static str],
    /// Blockkommentar (Anfang, Ende)
    pub block_comment: Option<(&'static str, &'static str)>,
    /// Parameter heißen „Name Typ“ (Go) statt „Typ Name“ (C) bzw. „Name: Typ“
    pub name_first: bool,
}

const C_LIKE: &[&str] = &["//"];
const HASH: &[&str] = &["#"];

pub static RUST: Lang = Lang {
    name: "Rust",
    keywords: &["as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while"],
    builtins: &["Option", "Some", "None", "Result", "Ok", "Err", "Vec", "String", "Box", "Rc", "Arc", "HashMap", "HashSet", "BTreeMap", "BTreeSet", "VecDeque", "Mutex", "RwLock", "Cell", "RefCell", "Path", "PathBuf", "Duration", "Instant", "Default", "Clone", "Copy", "Debug", "Display", "Iterator", "IntoIterator", "From", "Into", "AsRef", "println!", "print!", "eprintln!", "format!", "vec!", "panic!", "assert!", "assert_eq!", "debug_assert!", "write!", "writeln!", "todo!", "unimplemented!", "unreachable!", "matches!", "include_str!", "env!", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64", "bool", "char", "str"],
    members: &["unwrap", "unwrap_or", "unwrap_or_default", "unwrap_or_else", "expect", "map", "map_err", "and_then", "ok_or", "ok", "is_some", "is_none", "is_ok", "is_err", "iter", "iter_mut", "into_iter", "collect", "filter", "filter_map", "find", "any", "all", "enumerate", "zip", "rev", "skip", "take", "fold", "count", "sum", "min", "max", "len", "is_empty", "push", "pop", "insert", "remove", "get", "get_mut", "contains", "contains_key", "extend", "clear", "sort", "sort_by", "sort_by_key", "dedup", "retain", "clone", "to_string", "to_owned", "as_str", "as_ref", "as_mut", "trim", "split", "split_once", "starts_with", "ends_with", "replace", "chars", "bytes", "lines", "parse", "to_lowercase", "to_uppercase", "join", "lock", "borrow", "borrow_mut", "entry", "or_insert", "keys", "values", "first", "last"],
    line_comment: C_LIKE,
    block_comment: Some(("/*", "*/")),
    name_first: false,
};

pub static PYTHON: Lang = Lang {
    name: "Python",
    keywords: &["False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "self", "match", "case"],
    builtins: &["print", "len", "range", "enumerate", "zip", "map", "filter", "sorted", "reversed", "sum", "min", "max", "abs", "round", "open", "input", "isinstance", "issubclass", "hasattr", "getattr", "setattr", "type", "super", "iter", "next", "any", "all", "repr", "format", "id", "hash", "int", "float", "str", "bool", "list", "dict", "set", "tuple", "bytes", "object", "Exception", "ValueError", "TypeError", "KeyError", "IndexError", "RuntimeError", "staticmethod", "classmethod", "property", "dataclass", "__init__", "__name__", "__main__"],
    members: &["append", "extend", "insert", "remove", "pop", "clear", "index", "count", "sort", "reverse", "copy", "keys", "values", "items", "get", "update", "setdefault", "split", "rsplit", "join", "strip", "lstrip", "rstrip", "replace", "startswith", "endswith", "lower", "upper", "format", "find", "encode", "decode", "read", "write", "readlines", "close", "add", "discard", "union", "intersection"],
    line_comment: HASH,
    block_comment: None,
    name_first: false,
};

pub static JS: Lang = Lang {
    name: "JavaScript/TypeScript",
    keywords: &["async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do", "else", "export", "extends", "false", "finally", "for", "function", "if", "import", "from", "in", "instanceof", "let", "new", "null", "of", "return", "static", "super", "switch", "this", "throw", "true", "try", "typeof", "undefined", "var", "void", "while", "yield", "interface", "type", "enum", "implements", "private", "protected", "public", "readonly", "as", "keyof", "namespace", "declare", "abstract"],
    builtins: &["console", "document", "window", "Math", "JSON", "Object", "Array", "String", "Number", "Boolean", "Promise", "Map", "Set", "Date", "RegExp", "Error", "Symbol", "parseInt", "parseFloat", "isNaN", "setTimeout", "setInterval", "clearTimeout", "clearInterval", "fetch", "require", "module", "exports", "process", "string", "number", "boolean", "any", "unknown", "never", "void", "Record", "Partial", "Readonly", "Array", "useState", "useEffect", "useMemo", "useRef", "useCallback"],
    members: &["length", "push", "pop", "shift", "unshift", "slice", "splice", "concat", "join", "map", "filter", "reduce", "forEach", "find", "findIndex", "some", "every", "includes", "indexOf", "sort", "reverse", "keys", "values", "entries", "then", "catch", "finally", "toString", "split", "trim", "replace", "replaceAll", "startsWith", "endsWith", "toLowerCase", "toUpperCase", "substring", "log", "error", "warn", "info", "addEventListener", "removeEventListener", "querySelector", "querySelectorAll", "getElementById", "appendChild", "classList", "innerHTML", "textContent", "style", "value", "stringify", "parse", "get", "set", "has", "delete", "size"],
    line_comment: C_LIKE,
    block_comment: Some(("/*", "*/")),
    name_first: false,
};

pub static C: Lang = Lang {
    name: "C/C++",
    keywords: &["auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else", "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef", "union", "unsigned", "void", "volatile", "while", "bool", "true", "false", "nullptr", "class", "namespace", "using", "template", "typename", "public", "private", "protected", "virtual", "override", "new", "delete", "this", "try", "catch", "throw", "constexpr", "noexcept", "operator", "friend", "#include", "#define", "#ifdef", "#ifndef", "#endif", "#pragma"],
    builtins: &["printf", "scanf", "malloc", "calloc", "realloc", "free", "memcpy", "memset", "strlen", "strcpy", "strcmp", "fopen", "fclose", "fprintf", "NULL", "size_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t", "int32_t", "int64_t", "std", "string", "vector", "map", "unordered_map", "set", "unique_ptr", "shared_ptr", "make_unique", "make_shared", "cout", "cin", "endl", "move", "optional", "array", "pair"],
    members: &["size", "empty", "push_back", "emplace_back", "pop_back", "begin", "end", "insert", "erase", "find", "clear", "at", "front", "back", "data", "c_str", "substr", "length", "reserve", "resize", "get", "reset", "first", "second"],
    line_comment: C_LIKE,
    block_comment: Some(("/*", "*/")),
    name_first: false,
};

pub static CSHARP_JAVA: Lang = Lang {
    name: "C#/Java/Kotlin",
    keywords: &["abstract", "as", "base", "bool", "break", "case", "catch", "class", "const", "continue", "default", "do", "double", "else", "enum", "extends", "false", "final", "finally", "float", "for", "foreach", "if", "implements", "import", "in", "int", "interface", "internal", "is", "long", "namespace", "new", "null", "object", "override", "package", "private", "protected", "public", "readonly", "return", "sealed", "static", "string", "super", "switch", "this", "throw", "throws", "true", "try", "using", "var", "virtual", "void", "while", "async", "await", "fun", "val", "when", "data", "object", "companion", "lateinit"],
    builtins: &["Console", "String", "List", "Dictionary", "ArrayList", "HashMap", "Map", "Set", "Task", "Math", "Exception", "System", "Integer", "Object", "Optional", "Stream", "StringBuilder", "LINQ", "println", "WriteLine", "ToString", "listOf", "mapOf", "setOf", "mutableListOf"],
    members: &["Add", "Remove", "Count", "Length", "Contains", "ToList", "Where", "Select", "First", "FirstOrDefault", "Any", "OrderBy", "add", "remove", "size", "get", "put", "contains", "length", "equals", "toString", "stream", "map", "filter", "collect", "forEach", "isEmpty", "substring", "split", "trim", "append"],
    line_comment: C_LIKE,
    block_comment: Some(("/*", "*/")),
    name_first: false,
};

pub static GO: Lang = Lang {
    name: "Go",
    keywords: &["break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range", "return", "select", "struct", "switch", "type", "var", "nil", "true", "false"],
    builtins: &["append", "cap", "close", "copy", "delete", "len", "make", "new", "panic", "print", "println", "recover", "string", "int", "int64", "float64", "bool", "byte", "rune", "error", "any", "fmt", "Println", "Printf", "Sprintf", "Errorf", "os", "strings", "context", "http"],
    members: &["Println", "Printf", "Sprintf", "Error", "Close", "Read", "Write", "String", "Len", "Lock", "Unlock", "Done", "Contains", "Split", "Join", "TrimSpace", "HasPrefix", "HasSuffix"],
    line_comment: C_LIKE,
    block_comment: Some(("/*", "*/")),
    name_first: true,
};

pub static SHELL: Lang = Lang {
    name: "Shell",
    keywords: &["if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "until", "case", "esac", "function", "return", "local", "export", "readonly", "exit", "break", "continue"],
    builtins: &["echo", "printf", "read", "cd", "pwd", "ls", "cp", "mv", "rm", "mkdir", "grep", "sed", "awk", "cat", "head", "tail", "find", "xargs", "sort", "uniq", "wc", "curl", "chmod", "test", "source", "set", "shift", "trap"],
    members: &[],
    line_comment: HASH,
    block_comment: None,
    name_first: false,
};

pub static POWERSHELL: Lang = Lang {
    name: "PowerShell",
    keywords: &["if", "else", "elseif", "foreach", "for", "while", "do", "switch", "function", "param", "return", "try", "catch", "finally", "throw", "begin", "process", "end", "in", "true", "false", "null"],
    builtins: &["Write-Host", "Write-Output", "Write-Error", "Get-ChildItem", "Get-Content", "Set-Content", "Out-File", "Get-Item", "Remove-Item", "Copy-Item", "Move-Item", "New-Item", "Test-Path", "Join-Path", "Select-Object", "Where-Object", "ForEach-Object", "Sort-Object", "Measure-Object", "Invoke-WebRequest", "Start-Process", "Get-Process", "ConvertTo-Json", "ConvertFrom-Json"],
    members: &["Length", "Count", "Name", "FullName", "Split", "Trim", "Replace", "Contains", "ToLower", "ToUpper", "Substring"],
    line_comment: HASH,
    block_comment: Some(("<#", "#>")),
    name_first: false,
};

pub static PHP: Lang = Lang {
    name: "PHP",
    keywords: &["abstract", "array", "as", "break", "case", "catch", "class", "const", "continue", "default", "do", "echo", "else", "elseif", "extends", "false", "final", "finally", "for", "foreach", "function", "if", "implements", "include", "interface", "namespace", "new", "null", "private", "protected", "public", "require", "require_once", "return", "static", "switch", "this", "throw", "true", "try", "use", "while", "fn", "match"],
    builtins: &["count", "strlen", "str_replace", "explode", "implode", "array_map", "array_filter", "array_keys", "array_values", "in_array", "isset", "unset", "empty", "json_encode", "json_decode", "sprintf", "printf", "var_dump", "print_r", "trim", "substr", "strpos"],
    members: &[],
    line_comment: &["//", "#"],
    block_comment: Some(("/*", "*/")),
    name_first: false,
};

pub static RUBY_LUA: Lang = Lang {
    name: "Ruby/Lua",
    keywords: &["and", "begin", "break", "case", "class", "def", "do", "else", "elsif", "elseif", "end", "ensure", "false", "for", "function", "if", "in", "local", "module", "next", "nil", "not", "or", "redo", "rescue", "retry", "return", "self", "super", "then", "true", "unless", "until", "when", "while", "yield", "require"],
    builtins: &["puts", "print", "p", "require_relative", "attr_accessor", "attr_reader", "pairs", "ipairs", "tostring", "tonumber", "type", "table", "string", "math", "pcall", "error", "setmetatable"],
    members: &["each", "map", "select", "reject", "reduce", "length", "size", "push", "pop", "keys", "values", "include?", "to_s", "to_i", "insert", "remove", "concat", "format", "sub", "gsub", "find"],
    line_comment: &["#", "--"],
    block_comment: None,
    name_first: false,
};

pub static SQL: Lang = Lang {
    name: "SQL",
    keywords: &["SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "INSERT", "INTO", "VALUES", "UPDATE", "SET", "DELETE", "CREATE", "TABLE", "DROP", "ALTER", "INDEX", "VIEW", "JOIN", "LEFT", "RIGHT", "INNER", "OUTER", "ON", "GROUP", "BY", "ORDER", "HAVING", "LIMIT", "OFFSET", "AS", "DISTINCT", "NULL", "IS", "IN", "LIKE", "BETWEEN", "PRIMARY", "KEY", "FOREIGN", "REFERENCES", "UNIQUE", "DEFAULT", "CASE", "WHEN", "THEN", "ELSE", "END", "UNION", "ALL", "EXISTS"],
    builtins: &["COUNT", "SUM", "AVG", "MIN", "MAX", "COALESCE", "CAST", "NOW", "LOWER", "UPPER", "LENGTH", "SUBSTRING", "ROUND", "INTEGER", "TEXT", "VARCHAR", "BOOLEAN", "TIMESTAMP", "DATE"],
    members: &[],
    line_comment: &["--"],
    block_comment: Some(("/*", "*/")),
    name_first: false,
};

pub static WEB: Lang = Lang {
    name: "HTML/CSS",
    keywords: &["html", "head", "body", "div", "span", "section", "header", "footer", "nav", "main", "article", "button", "input", "form", "label", "select", "option", "textarea", "img", "script", "style", "link", "meta", "title", "table", "thead", "tbody", "tr", "td", "th", "ul", "ol", "li", "class", "href", "src", "alt", "type", "id"],
    builtins: &["display", "flex", "grid", "position", "absolute", "relative", "fixed", "margin", "padding", "border", "border-radius", "background", "background-color", "color", "font-size", "font-weight", "font-family", "width", "height", "max-width", "min-height", "align-items", "justify-content", "gap", "overflow", "opacity", "transition", "transform", "z-index", "box-shadow", "cursor", "none", "block", "inline-block", "center", "auto", "important"],
    members: &[],
    line_comment: &[],
    block_comment: Some(("/*", "*/")),
    name_first: false,
};

pub static GENERIC: Lang = Lang { name: "Text", keywords: &[], builtins: &[], members: &[], line_comment: &["//", "#"], block_comment: Some(("/*", "*/")), name_first: false };

/// Sprache zur Dateiendung (ohne Punkt, klein).
pub fn for_ext(ext: &str) -> &'static Lang {
    match ext {
        "rs" => &RUST,
        "py" | "pyw" | "pyi" => &PYTHON,
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "mts" | "vue" | "svelte" => &JS,
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "ino" => &C,
        "cs" | "java" | "kt" | "kts" | "scala" | "dart" | "swift" => &CSHARP_JAVA,
        "go" => &GO,
        "sh" | "bash" | "zsh" | "fish" => &SHELL,
        "ps1" | "psm1" | "psd1" => &POWERSHELL,
        "php" => &PHP,
        "rb" | "lua" => &RUBY_LUA,
        "sql" => &SQL,
        "html" | "htm" | "xml" | "css" | "scss" | "less" | "svg" => &WEB,
        _ => &GENERIC,
    }
}

/// Kommentare und Zeichenketten durch Leerzeichen ersetzen (Länge und Zeilen bleiben gleich), damit darin
/// keine Bezeichner erkannt werden.
pub fn blank_comments_strings(text: &str, lang: &Lang) -> String {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in out.iter_mut().take(to).skip(from) {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    while i < b.len() {
        let rest = &text[i..];
        if lang.line_comment.iter().any(|lc| rest.starts_with(lc)) {
            let end = rest.find('\n').map_or(b.len(), |e| i + e);
            blank(&mut out, i, end);
            i = end;
            continue;
        }
        if let Some((open, close)) = lang.block_comment {
            if rest.starts_with(open) {
                let end = rest[open.len()..].find(close).map_or(b.len(), |e| i + open.len() + e + close.len());
                blank(&mut out, i, end);
                i = end;
                continue;
            }
        }
        let c = b[i];
        if c == b'"' || c == b'`' || (c == b'\'' && lang.name != "Rust") {
            let mut j = i + 1;
            while j < b.len() && b[j] != c && !(b[j] == b'\n' && c != b'`') {
                if b[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(b.len());
            // Anführungszeichen stehen lassen, Inhalt leeren
            blank(&mut out, i + 1, end.saturating_sub(1).max(i + 1));
            i = end;
            continue;
        }
        // UTF-8: ganze Zeichen überspringen
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}
