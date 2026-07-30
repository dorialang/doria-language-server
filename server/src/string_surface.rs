pub(crate) struct StringMember {
    pub(crate) name: &'static str,
    pub(crate) signature: &'static str,
    pub(crate) documentation: &'static str,
}

macro_rules! string_method {
    ($name:literal, $signature:literal, $documentation:literal) => {
        StringMember {
            name: $name,
            signature: $signature,
            documentation: $documentation,
        }
    };
}

pub(crate) const STRING_PROPERTIES: &[StringMember] = &[
    StringMember {
        name: "length",
        signature: "int $length",
        documentation: "Returns the number of Unicode extended grapheme clusters.",
    },
    StringMember {
        name: "byteLength",
        signature: "int $byteLength",
        documentation: "Returns the exact UTF-8 byte count.",
    },
    StringMember {
        name: "isEmpty",
        signature: "bool $isEmpty",
        documentation: "Reports whether the string contains zero UTF-8 bytes.",
    },
    StringMember {
        name: "bytes",
        signature: "Bytes $bytes",
        documentation: "Returns an owned mutable copy of the string's UTF-8 bytes.",
    },
];

pub(crate) const STRING_COMPANION_METHODS: &[StringMember] = &[
    string_method!(
        "trim",
        "String::trim(string $text): string",
        "Removes Unicode whitespace from both ends."
    ),
    string_method!(
        "trimStart",
        "String::trimStart(string $text): string",
        "Removes Unicode whitespace from the beginning."
    ),
    string_method!(
        "trimEnd",
        "String::trimEnd(string $text): string",
        "Removes Unicode whitespace from the end."
    ),
    string_method!(
        "lower",
        "String::lower(string $text): string",
        "Applies locale-independent default Unicode lowercase mapping."
    ),
    string_method!(
        "upper",
        "String::upper(string $text): string",
        "Applies locale-independent default Unicode uppercase mapping."
    ),
    string_method!(
        "lowerFirst",
        "String::lowerFirst(string $text): string",
        "Lowercases the first extended grapheme cluster without normalization."
    ),
    string_method!(
        "upperFirst",
        "String::upperFirst(string $text): string",
        "Uppercases the first extended grapheme cluster without normalization."
    ),
    string_method!(
        "contains",
        "String::contains(string $text, string $needle): bool",
        "Tests for a case-sensitive, grapheme-boundary-aligned literal match."
    ),
    string_method!(
        "startsWith",
        "String::startsWith(string $text, string $prefix): bool",
        "Tests for a case-sensitive literal prefix on grapheme boundaries."
    ),
    string_method!(
        "endsWith",
        "String::endsWith(string $text, string $suffix): bool",
        "Tests for a case-sensitive literal suffix on grapheme boundaries."
    ),
    string_method!(
        "equalsIgnoreCase",
        "String::equalsIgnoreCase(string $left, string $right): bool",
        "Compares with full default Unicode case folding and no normalization."
    ),
    string_method!(
        "containsIgnoreCase",
        "String::containsIgnoreCase(string $text, string $needle): bool",
        "Tests for a grapheme-boundary-aligned match using full default Unicode case folding."
    ),
    string_method!(
        "startsWithIgnoreCase",
        "String::startsWithIgnoreCase(string $text, string $prefix): bool",
        "Tests for a folded prefix while preserving original grapheme boundaries."
    ),
    string_method!(
        "endsWithIgnoreCase",
        "String::endsWithIgnoreCase(string $text, string $suffix): bool",
        "Tests for a folded suffix while preserving original grapheme boundaries."
    ),
    string_method!(
        "indexOf",
        "String::indexOf(string $text, string $needle): ?int",
        "Returns the first matching grapheme index, or null."
    ),
    string_method!(
        "lastIndexOf",
        "String::lastIndexOf(string $text, string $needle): ?int",
        "Returns the final matching grapheme index, or null."
    ),
    string_method!(
        "indexOfIgnoreCase",
        "String::indexOfIgnoreCase(string $text, string $needle): ?int",
        "Returns the first folded match as an index in the original grapheme sequence, or null."
    ),
    string_method!(
        "lastIndexOfIgnoreCase",
        "String::lastIndexOfIgnoreCase(string $text, string $needle): ?int",
        "Returns the final folded match as an index in the original grapheme sequence, or null."
    ),
    string_method!(
        "countOccurrences",
        "String::countOccurrences(string $text, string $needle): int",
        "Counts non-overlapping grapheme-boundary matches from left to right."
    ),
    string_method!(
        "replace",
        "String::replace(string $text, string $search, string $replacement): string",
        "Replaces all non-overlapping literal matches on grapheme boundaries."
    ),
    string_method!(
        "split",
        "String::split(string $text, string $separator): List<string>",
        "Splits on literal grapheme-boundary matches and preserves empty fields."
    ),
    string_method!(
        "join",
        "String::join(string $separator, List<string> $values): string",
        "Joins a readonly list without consuming it."
    ),
    string_method!(
        "slice",
        "String::slice(string $text, int $start, ?int $length = null): string",
        "Returns a clamped slice measured in extended grapheme clusters."
    ),
    string_method!(
        "repeat",
        "String::repeat(string $text, int $count): string",
        "Repeats the string; a negative count panics."
    ),
    string_method!(
        "padStart",
        "String::padStart(string $text, int $length, string $padding): string",
        "Pads the beginning to an exact grapheme length."
    ),
    string_method!(
        "padEnd",
        "String::padEnd(string $text, int $length, string $padding): string",
        "Pads the end to an exact grapheme length."
    ),
    string_method!(
        "fromBytes",
        "String::fromBytes(Bytes $bytes): ?string",
        "Copies valid UTF-8 bytes into a string and returns null for invalid UTF-8."
    ),
];

pub(crate) fn string_property(name: &str) -> Option<&'static StringMember> {
    STRING_PROPERTIES.iter().find(|member| member.name == name)
}

pub(crate) fn string_companion_method(name: &str) -> Option<&'static StringMember> {
    STRING_COMPANION_METHODS
        .iter()
        .find(|member| member.name == name)
}
