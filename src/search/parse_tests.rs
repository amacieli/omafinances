use super::fixture::today;
use super::*;
use chrono::NaiveDate;

fn p(s: &str) -> Search {
    parse(s, today()).unwrap_or_else(|e| panic!("parse {:?}: {}", s, e))
}

fn e(s: &str) -> ParseError {
    parse(s, today()).expect_err(s)
}

fn canon(s: &str) -> String {
    p(s).canonical()
}

fn kind(s: &str) -> TermKind {
    let mut t = p(s).terms;
    assert_eq!(t.len(), 1, "{}", s);
    t.remove(0).kind
}

// ---- fields and aliases -------------------------------------------------------------------------

#[test]
fn every_field_alias_is_accepted_case_insensitively() {
    for (name, field) in Field::NAMES {
        let value = match field {
            Field::Id => "5",
            Field::Date => "2026",
            Field::Amount => "5",
            Field::Is => "pending",
            Field::Sort => "date",
            _ => "x",
        };
        for spelled in [name.to_string(), name.to_uppercase()] {
            let s = p(&format!("{}:{}", spelled, value));
            if *field == Field::Sort {
                assert_eq!(s.sort, Some((SortKey::Date, Dir::Asc)));
                assert!(s.terms.is_empty());
            } else {
                assert_eq!(s.terms.len(), 1);
                assert_eq!(s.terms[0].parts().0, Some(*field), "{}", spelled);
                assert_eq!(s.terms[0].canonical(), format!("{}:{}", field.short(), value));
            }
        }
    }
}

#[test]
fn free_text_and_payee_values_are_lowercased_substrings() {
    assert_eq!(kind("Amazon"), TermKind::Text("amazon".into()));
    assert_eq!(kind("p:AmAzOn"), TermKind::Payee(vec!["amazon".into()]));
    assert_eq!(p("a b c").terms.len(), 3);
    assert!(p("").terms.is_empty());
    assert!(p("   \t ").terms.is_empty());
}

#[test]
fn or_lists_and_negation() {
    assert_eq!(kind("p:amazon,costco"), TermKind::Payee(vec!["amazon".into(), "costco".into()]));
    let s = p("-p:amazon -c:none foo");
    assert!(s.terms[0].neg && s.terms[1].neg && !s.terms[2].neg);
    assert_eq!(s.canonical(), "-p:amazon -c:none foo");
    assert_eq!(kind("is:income,pending"), TermKind::Is(vec![IsKind::Income, IsKind::Pending]));
    assert_eq!(kind("id:1,2,3"), TermKind::Id(vec![1, 2, 3]));
    assert!(p("-\"a b\"").terms[0].neg);
}

#[test]
fn quoting_and_escapes() {
    assert_eq!(kind("\"harris teeter\""), TermKind::Text("harris teeter".into()));
    assert_eq!(kind("p:\"harris teeter\""), TermKind::Payee(vec!["harris teeter".into()]));
    assert_eq!(kind("p:\"a,b\",c"), TermKind::Payee(vec!["a,b".into(), "c".into()]));
    assert_eq!(kind("\"say \\\"hi\\\"\""), TermKind::Text("say \"hi\"".into()));
    assert_eq!(kind("p:a\\\"b"), TermKind::Payee(vec!["a\"b".into()]));
    assert_eq!(kind("\"back\\\\slash\""), TermKind::Text("back\\slash".into()));
    // a quoted word is text even when it looks like an amount or a field
    assert_eq!(kind("\"100\""), TermKind::Text("100".into()));
    assert_eq!(kind("\"p:x\""), TermKind::Text("p:x".into()));
    assert_eq!(kind("\"-x\""), TermKind::Text("-x".into()));
    // quote in the middle of a token
    assert_eq!(kind("foo\"bar baz\""), TermKind::Text("foobar baz".into()));
}

#[test]
fn categories() {
    assert_eq!(kind("c:none"), TermKind::Category(vec![CatSpec::None]));
    assert_eq!(kind("c:NONE"), TermKind::Category(vec![CatSpec::None]));
    assert_eq!(kind("c:home"), TermKind::Category(vec![CatSpec::Path(vec!["home".into()])]));
    assert_eq!(kind("c:Home/Maint"), TermKind::Category(vec![CatSpec::Path(vec!["home".into(), "maint".into()])]));
    assert_eq!(kind("c:\"Home/Sub 0\""), TermKind::Category(vec![CatSpec::Path(vec!["home".into(), "sub 0".into()])]));
    assert_eq!(kind("c:home,salary/job,none").clone(), TermKind::Category(vec![
        CatSpec::Path(vec!["home".into()]),
        CatSpec::Path(vec!["salary".into(), "job".into()]),
        CatSpec::None,
    ]));
    assert_eq!(canon("c:\"Home/Sub 0\""), "c:\"home/sub 0\"");
}

#[test]
fn dates_absolute_forms() {
    let d = |s: &str| match kind(s) {
        TermKind::Date(v) => (v[0].lo, v[0].hi),
        k => panic!("{:?}", k),
    };
    assert_eq!(d("d:2026-10-05"), (Some(20261005), Some(20261005)));
    assert_eq!(d("d:2026-10"), (Some(20261001), Some(20261031)));
    assert_eq!(d("d:2026-02"), (Some(20260201), Some(20260228)));
    assert_eq!(d("d:2024-02"), (Some(20240201), Some(20240229)));
    assert_eq!(d("d:2026"), (Some(20260101), Some(20261231)));
    assert_eq!(d("d:2026-10-01..2026-11-15"), (Some(20261001), Some(20261115)));
    assert_eq!(d("d:2026-10-01.."), (Some(20261001), None));
    assert_eq!(d("d:..2026-03-31"), (None, Some(20260331)));
    assert_eq!(d("d:2026-01..2026-03"), (Some(20260101), Some(20260331)));
    assert_eq!(d("d:>2026-01-01"), (Some(20260102), None));
    assert_eq!(d("d:>=2026-01-01"), (Some(20260101), None));
    assert_eq!(d("d:<2026-01-01"), (None, Some(20251231)));
    assert_eq!(d("d:<=2026-01-01"), (None, Some(20260101)));
    assert_eq!(d("d:>2026-01"), (Some(20260201), None));
    assert_eq!(d("d:<2026"), (None, Some(20251231)));
    assert_eq!(d("d:=2026-10-05"), (Some(20261005), Some(20261005)));
    assert_eq!(d("date:2026-10-05,2026-11"), (Some(20261005), Some(20261005)));
}

#[test]
fn dates_presets_and_relative_use_the_injected_today() {
    // today = Thursday 2026-01-08
    let d = |s: &str| match kind(s) {
        TermKind::Date(v) => (v[0].lo, v[0].hi),
        k => panic!("{:?}", k),
    };
    assert_eq!(d("d:today"), (Some(20260108), Some(20260108)));
    assert_eq!(d("d:yesterday"), (Some(20260107), Some(20260107)));
    assert_eq!(d("d:this-week"), (Some(20260105), Some(20260111)));
    assert_eq!(d("d:last-week"), (Some(20251229), Some(20260104)));
    assert_eq!(d("d:this-month"), (Some(20260101), Some(20260131)));
    assert_eq!(d("d:last-month"), (Some(20251201), Some(20251231)));
    assert_eq!(d("d:this-quarter"), (Some(20260101), Some(20260331)));
    assert_eq!(d("d:last-quarter"), (Some(20251001), Some(20251231)));
    assert_eq!(d("d:ytd"), (Some(20260101), Some(20260108)));
    assert_eq!(d("d:this-year"), (Some(20260101), Some(20261231)));
    assert_eq!(d("d:last-year"), (Some(20250101), Some(20251231)));
    assert_eq!(d("d:THIS-MONTH"), (Some(20260101), Some(20260131)));
    // relative: `>30d` newer than 30 days ago, `<30d` older than that, `7d` the last 7 days
    assert_eq!(d("d:>30d"), (Some(20251210), None));
    assert_eq!(d("d:>=30d"), (Some(20251209), None));
    assert_eq!(d("d:<30d"), (None, Some(20251208)));
    assert_eq!(d("d:<=30d"), (None, Some(20251209)));
    assert_eq!(d("d:7d"), (Some(20260102), Some(20260108)));
    assert_eq!(d("d:1w"), (Some(20260102), Some(20260108)));
    assert_eq!(d("d:3m"), (Some(20251009), Some(20260108)));
    assert_eq!(d("d:1y"), (Some(20250109), Some(20260108)));
    // other days of the quarter / year boundaries
    let s = parse("d:this-quarter d:last-quarter d:last-week", NaiveDate::from_ymd_opt(2026, 11, 20).unwrap()).unwrap();
    let r = |i: usize| match &s.terms[i].kind {
        TermKind::Date(v) => (v[0].lo, v[0].hi),
        _ => unreachable!(),
    };
    assert_eq!(r(0), (Some(20261001), Some(20261231)));
    assert_eq!(r(1), (Some(20260701), Some(20260930)));
    assert_eq!(r(2), (Some(20261109), Some(20261115)));
    // month arithmetic clamps the day: 3 months before 2026-05-31 is 2026-02-28
    let s = parse("d:>=3m", NaiveDate::from_ymd_opt(2026, 5, 31).unwrap()).unwrap();
    assert_eq!(s.canonical(), "d:>=3m");
    match &s.terms[0].kind {
        TermKind::Date(v) => assert_eq!(v[0].lo, Some(20260228)),
        _ => unreachable!(),
    }
}

#[test]
fn amounts() {
    let a = |s: &str| match kind(s) {
        TermKind::Amount(v) => v,
        k => panic!("{:?}", k),
    };
    assert_eq!(a("amount:>50"), vec![AmountSpec::Gt(50.0)]);
    assert_eq!(a("amt:>=50"), vec![AmountSpec::Ge(50.0)]);
    assert_eq!(a("amount:<20"), vec![AmountSpec::Lt(20.0)]);
    assert_eq!(a("amount:<=20.5"), vec![AmountSpec::Le(20.5)]);
    assert_eq!(a("amount:100"), vec![AmountSpec::Eq(100.0)]);
    assert_eq!(a("amount:=100"), vec![AmountSpec::Eq(100.0)]);
    assert_eq!(a("amount:10..50"), vec![AmountSpec::Range(Some(10.0), Some(50.0))]);
    assert_eq!(a("amount:10.."), vec![AmountSpec::Range(Some(10.0), None)]);
    assert_eq!(a("amount:..50"), vec![AmountSpec::Range(None, Some(50.0))]);
    assert_eq!(a("amount:>$1,200"), vec![AmountSpec::Gt(1200.0)]);
    assert_eq!(a("amount:$1,200.50"), vec![AmountSpec::Eq(1200.5)]);
    assert_eq!(a("amount:$1,000..$2,500"), vec![AmountSpec::Range(Some(1000.0), Some(2500.0))]);
    assert_eq!(a("amount:.5"), vec![AmountSpec::Eq(0.5)]);
    // bare tokens
    assert_eq!(a(">50"), vec![AmountSpec::Gt(50.0)]);
    assert_eq!(a(">=50"), vec![AmountSpec::Ge(50.0)]);
    assert_eq!(a("<20"), vec![AmountSpec::Lt(20.0)]);
    assert_eq!(a("10..50"), vec![AmountSpec::Range(Some(10.0), Some(50.0))]);
    assert_eq!(a("100"), vec![AmountSpec::Eq(100.0)]);
    assert_eq!(a("=100"), vec![AmountSpec::Eq(100.0)]);
    assert_eq!(a(">$1,200"), vec![AmountSpec::Gt(1200.0)]);
    assert_eq!(a("$5"), vec![AmountSpec::Eq(5.0)]);
    assert_eq!(a("1,200"), vec![AmountSpec::Eq(1200.0)]);
    // lists: a comma before exactly three digits is a thousands separator; anything else splits
    assert_eq!(a("amount:10,20"), vec![AmountSpec::Eq(10.0), AmountSpec::Eq(20.0)]);
    assert_eq!(a("amount:1,200"), vec![AmountSpec::Eq(1200.0)]);
    assert_eq!(a("amount:100,=200"), vec![AmountSpec::Eq(100.0), AmountSpec::Eq(200.0)]);
    assert_eq!(a("amount:>5,<2"), vec![AmountSpec::Gt(5.0), AmountSpec::Lt(2.0)]);
    // not amounts: free text
    assert_eq!(kind("7-eleven"), TermKind::Text("7-eleven".into()));
    assert_eq!(kind("1,2"), TermKind::Text("1,2".into()));
    assert_eq!(kind("3.5.1"), TermKind::Text("3.5.1".into()));
    // a bare leading minus is negation
    let s = p("-20");
    assert!(s.terms[0].neg);
    assert_eq!(s.terms[0].kind, TermKind::Amount(vec![AmountSpec::Eq(20.0)]));
}

#[test]
fn is_and_sort() {
    for k in IsKind::ALL {
        assert_eq!(kind(&format!("is:{}", k.name())), TermKind::Is(vec![k]));
    }
    assert_eq!(p("sort:amount").sort, Some((SortKey::Amount, Dir::Asc)));
    assert_eq!(p("sort:-amount").sort, Some((SortKey::Amount, Dir::Desc)));
    for k in SortKey::ALL {
        assert_eq!(p(&format!("x sort:-{}", k.name())).sort, Some((k, Dir::Desc)));
    }
    assert_eq!(p("p:a sort:-date").terms.len(), 1);
    assert_eq!(canon("sort:-payee p:a"), "p:a sort:-payee");
    assert_eq!(p("p:a").sort, None);
    assert_eq!(p("p:a sort:-date").sort_text().as_deref(), Some("-date"));
}

// ---- errors ----------------------------------------------------------------------------------

#[test]
fn unknown_field_has_position_and_suggestion() {
    let err = e("p:ok pyee:x");
    assert_eq!((err.pos, err.len), (5, 4));
    assert_eq!(err.message, "unknown field \"pyee\"; did you mean \"p\"?");
    let err = e("acount:x");
    assert!(err.message.contains("did you mean \"a\""), "{}", err.message);
    let err = e("zzzz:x");
    assert!(err.message.starts_with("unknown field \"zzzz\"") && !err.message.contains("did you mean"), "{}", err.message);
    // negated terms point at the name, not the minus
    let err = e("-pyee:x");
    assert_eq!((err.pos, err.len), (1, 4));
    // byte (not char) positions after multibyte text
    let err = e("é pyee:x");
    assert_eq!((err.pos, err.len), (3, 4));
    let err = e(":x");
    assert_eq!(err.pos, 0);
}

#[test]
fn value_errors_point_at_the_value() {
    let err = e("p:a is:recuring");
    assert_eq!((err.pos, err.len), (7, 8));
    assert!(err.message.contains("did you mean \"recurring\""), "{}", err.message);

    let err = e("d:2026-13-01");
    assert_eq!((err.pos, err.len), (2, 10));
    assert!(err.message.contains("not a real date"), "{}", err.message);
    assert!(e("d:2026-02-30").message.contains("not a real date"));
    assert!(e("d:2026-00").message.contains("not a real date"));

    let err = e("d:thismonth");
    assert!(err.message.contains("did you mean \"this-month\""), "{}", err.message);
    assert!(e("d:tod").message.contains("did you mean \"today\""));
    assert!(e("d:30").message.contains("bad date"));
    assert!(e("d:..").message.contains("bad date"));
    assert!(e("d:2026-02-01..2026-01-01").message.contains("starts after its end"));
    assert!(e("d:=7d").message.contains("bad date"));
    assert!(e("d:99999999d").message.contains("bad date") || e("d:99999999d").message.contains("far"));
    assert!(e("d:100001d").message.contains("far back"));

    let err = e("amount:abc");
    assert_eq!((err.pos, err.len), (7, 3));
    assert!(err.message.contains("bad amount"));
    assert!(e("amount:10..5").message.contains("starts above"));
    assert!(e("amount:>").message.contains("bad amount"));
    assert!(e("amount:-5").message.contains("bad amount"));
    assert_eq!(p("amount:1,23").terms[0].canonical(), "amount:=1,=23");
    assert!(e("amount:..").message.contains("bad amount"));

    assert!(e("id:abc").message.contains("bad id"));
    assert!(e("id:-5").message.contains("bad id"));
    assert!(e("sort:dat").message.contains("did you mean \"date\""));
    assert!(e("sort:-").message.contains("unknown sort key"));
    assert!(e("c:/").message.contains("empty category path"));
}

#[test]
fn bare_amount_syntax_errors_and_structure_errors() {
    // committed amount syntax errors rather than turning into text
    assert_eq!(kind(">abc"), TermKind::Text(">abc".into()));
    let err = e(">");
    assert!(err.message.contains("bad amount"));
    let err = e("a >=");
    assert_eq!((err.pos, err.len), (2, 2));
    assert!(e("10..5").message.contains("starts above"));

    let err = e("p:");
    assert_eq!((err.pos, err.len), (1, 1));
    assert!(err.message.contains("missing value after \"p:\""));
    assert!(e("p:\"\"").message.contains("missing value"));
    let err = e("p:a,,b");
    assert_eq!((err.pos, err.len), (4, 1));
    assert!(e("p:a,").message.contains("empty value"));
    assert!(e("p:,a").message.contains("empty value"));
    assert!(e("\"\"").message.contains("empty"));

    let err = e("\"abc");
    assert_eq!((err.pos, err.len), (0, 4));
    assert!(err.message.contains("unterminated quote"));
    let err = e("a \"b c");
    assert_eq!((err.pos, err.len), (2, 4));
    assert!(e("p:\"x").message.contains("unterminated"));

    let err = e("a -");
    assert_eq!((err.pos, err.len), (2, 1));
    assert!(err.message.contains("nothing to negate"));
    assert!(e("-sort:date").message.contains("cannot be negated"));
    let err = e("sort:date sort:amount");
    assert_eq!(err.pos, 10);
    assert!(err.message.contains("more than once"));
    assert!(e("sort:date,amount").message.contains("single value"));
}

#[test]
fn error_positions_are_inside_the_input() {
    for s in ["p:", "x p:", "\"", "-", "é\"", "pyee:é", "d:é", "amount:é", "is:é", "p:,", "ééé:x", "a,b:é"] {
        if let Err(err) = parse(s, today()) {
            assert!(err.pos < s.len() && err.len >= 1 && err.pos + err.len <= s.len(), "{:?} -> {:?}", s, err);
        }
    }
}

// ---- canonical text and fuzzing -------------------------------------------------------------

#[test]
fn canonical_normalises_and_round_trips() {
    for (input, want) in [
        ("Payee:Amazon   CAT:Home/Maint", "p:amazon c:home/maint"),
        ("\"harris teeter\" -is:pending", "\"harris teeter\" -is:pending"),
        ("amt:>=$1,200", "amount:>=1200"),
        ("100", "amount:100"),
        ("10..50", "amount:10..50"),
        ("amount:10,20", "amount:=10,=20"),
        ("amount:10..50,60..70", "amount:$10..50,$60..70"),
        ("d:This-Month", "d:this-month"),
        ("note:\"a, b\"", "n:\"a, b\""),
        ("p:\"say \\\"hi\\\"\"", "p:\"say \\\"hi\\\"\""),
        ("\"-x\" \"100\" \"p:x\"", "\"-x\" \"100\" \"p:x\""),
        ("c:\"Home/Sub 0\",none", "c:\"home/sub 0\",none"),
        ("coca-cola", "coca-cola"),
        ("sort:-amount a", "a sort:-amount"),
        ("", ""),
    ] {
        let s = p(input);
        assert_eq!(s.canonical(), want, "{}", input);
        assert_eq!(p(&s.canonical()), s, "round trip {}", input);
    }
}

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.next() as usize % items.len()]
    }
}

const PIECES: &[&str] = &[
    "p:", "a:", "c:", "d:", "n:", "lm:", "id:", "is:", "sort:", "amount:", "amt:", "-", ",", ":", "\"", "\\", "\\\"", " ", "  ", "\t", "..", ">", "<", ">=", "<=",
    "=", "$", "1,200", "50", "0.5", "2026", "2026-10", "2026-10-05", "this-month", "30d", "3m", "1y", "amazon", "home", "home/sub", "none", "pending", "income",
    "date", "-amount", "é", "€", "日本", "😀", "\u{200b}", "\u{a0}", "x", "/", "*", "(", "9999999999999999999", "d:>30d", "99999d", "p:\"a b\"", "c:\"x/y z\"",
];

fn random_input(r: &mut Lcg) -> String {
    let n = r.next() % 9;
    (0..n).map(|_| *r.pick(PIECES)).collect()
}

fn random_bytes_string(r: &mut Lcg) -> String {
    let n = r.next() % 24;
    (0..n).map(|_| char::from_u32(r.next() % 0x3000).unwrap_or('?')).collect()
}

#[test]
fn never_panics_and_round_trips_on_random_input() {
    let mut r = Lcg(0x1234_5678_9abc_def0);
    let mut ok = 0;
    let mut bad = 0;
    for i in 0..60_000 {
        let s = if i % 4 == 0 { random_bytes_string(&mut r) } else { random_input(&mut r) };
        match parse(&s, today()) {
            Ok(search) => {
                ok += 1;
                let c = search.canonical();
                let again = parse(&c, today()).unwrap_or_else(|e| panic!("canonical {:?} of {:?} does not parse: {}", c, s, e));
                assert_eq!(again, search, "round trip of {:?} via {:?}", s, c);
                assert_eq!(again.canonical(), c);
            }
            Err(err) => {
                bad += 1;
                assert!(err.pos <= s.len() && s.is_char_boundary(err.pos) && err.len >= 1 && !err.message.is_empty(), "{:?} -> {:?}", s, err);
            }
        }
    }
    assert!(ok > 2000 && bad > 2000, "fuzz mix too lopsided: ok {} bad {}", ok, bad);
}

#[test]
fn never_panics_on_extreme_input() {
    let long = "p:a,".repeat(5000);
    let _ = parse(&long, today());
    let _ = parse(&"\"".repeat(1001), today());
    let _ = parse(&"-".repeat(1000), today());
    let _ = parse(&"\\".repeat(1000), today());
    let _ = parse(&format!("d:{}d", "9".repeat(400)), today());
    let _ = parse(&format!("amount:{}", "9".repeat(400)), today());
    let _ = parse(&format!("amount:{}..{}", "1".repeat(30), "2".repeat(30)), today());
    let _ = parse("d:0000-01-01 d:0000 d:9999-12-31.. d:>9999-12-31 d:<0001-01-01", today());
    let _ = parse("d:..9999 d:>9999 d:<0001", NaiveDate::from_ymd_opt(1, 1, 1).unwrap());
    let _ = parse("d:last-year d:yesterday d:last-week d:3m d:1y", NaiveDate::from_ymd_opt(1, 1, 1).unwrap());
    let _ = parse("d:this-year d:tomorrow d:5d", NaiveDate::MAX);
}
