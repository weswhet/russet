use fancy_regex::RegexBuilder;

#[test]
fn python_lowercase_and_scalar_advancement() {
    for (pattern, text, expected) in [
        (r"(?i:(i)\1)x", "iİx", true),
        (r"(?i:(i)\1)", "iı", false),
        (r"(?i:(s)\1)", "sſ", false),
        (r"(?i:(Σ)\1)", "Σς", false),
        (r"(?i:(µ)\1)", "µΜ", false),
        (r"(?i:(k)\1)x", "kKx", true),
        (r"(?i:(K)\1)x", "Kkx", true),
        (r"(?i:()\1)x", "x", true),
        (r"(?i:(i|ik)\1)x", "ikİKx", true),
        (r"(?i:(a)(?a:\1))", "aA", true),
        (r"(?i:(é)(?a:\1))", "éÉ", false),
        (r"(?i:(k)(?a:\1))", "kK", false),
    ] {
        let re = RegexBuilder::new(pattern)
            .python_backreferences(true)
            .build()
            .unwrap();
        assert_eq!(re.is_match(text).unwrap(), expected, "{pattern} {text}");
    }
}

#[test]
fn default_engine_retains_upstream_semantics() {
    assert!(RegexBuilder::new(r"(?a:(a)\1)").build().is_err());
    assert!(RegexBuilder::new(r"(?i:(Σ)\1)")
        .build()
        .unwrap()
        .is_match("Σς")
        .unwrap());
    assert!(!RegexBuilder::new(r"(?i:(k)\1)")
        .build()
        .unwrap()
        .is_match("kK")
        .unwrap());
}
