use super::*;

#[test]
fn names_numeric_weekdays_so_either_numbering_reads_them_the_same() {
    assert_eq!(to_named_weekdays("0 7 * * 5"), "0 7 * * FRI");
    assert_eq!(to_named_weekdays("0 7 * * 0"), "0 7 * * SUN");
    assert_eq!(to_named_weekdays("0 7 * * 7"), "0 7 * * SUN");
    assert_eq!(to_named_weekdays("0 9 * * 1-5"), "0 9 * * MON-FRI");
    assert_eq!(
        to_named_weekdays("30 18 * * 1,3,5"),
        "30 18 * * MON,WED,FRI"
    );
    assert_eq!(to_named_weekdays("0 9 * * 1-5/2"), "0 9 * * MON,WED,FRI");
    assert_eq!(to_named_weekdays("0 9 * * */3"), "0 9 * * SUN,WED,SAT");
}

#[test]
fn leaves_names_wildcards_and_other_fields_alone() {
    assert_eq!(to_named_weekdays("0 7 * * *"), "0 7 * * *");
    assert_eq!(to_named_weekdays("0 7 1 5 *"), "0 7 1 5 *");
    assert_eq!(to_named_weekdays("0 7 * * mon-fri"), "0 7 * * MON-FRI");
    assert_eq!(to_named_weekdays("0 7 * * ?"), "0 7 * * ?");
}

#[test]
fn reads_the_weekday_of_a_six_field_cron_after_its_seconds() {
    assert_eq!(to_named_weekdays("0 0 7 * * 5"), "0 0 7 * * FRI");
    assert_eq!(to_named_weekdays("not a cron"), "not a cron");
}

#[test]
fn the_named_form_is_one_the_scheduler_accepts() {
    for expression in ["0 7 * * 5", "0 9 * * 1-5", "0 9 * * */3", "0 7 * * 0"] {
        let named = to_named_weekdays(expression);
        assert!(
            teeny_cadence::calendar::is_cron_expression(&named),
            "{named}"
        );
    }
}

#[test]
fn names_a_step_over_a_named_range_and_a_wrapping_named_range() {
    assert_eq!(
        named_weekdays("0 9 * * MON-FRI/2").unwrap(),
        "0 9 * * MON,WED,FRI"
    );
    assert_eq!(
        named_weekdays("0 9 * * mon/2").unwrap(),
        "0 9 * * MON,WED,FRI"
    );
    assert_eq!(
        named_weekdays("0 9 * * SAT-SUN").unwrap(),
        "0 9 * * SAT,SUN"
    );
    assert_eq!(named_weekdays("0 9 * * MON-5").unwrap(), "0 9 * * MON-FRI");
}

#[test]
fn refuses_a_weekday_that_cannot_be_written_with_names() {
    for weekday in ["5#2", "L", "5L", "1W", "FOO", "MONDAY", "*/0", "8", "1-9"] {
        let expression = format!("0 9 * * {weekday}");
        let error = named_weekdays(&expression).unwrap_err();
        assert_eq!(error.field, weekday, "{expression}");
        let said = error.to_string();
        assert!(said.contains(weekday), "{said}");
        assert!(said.contains("day names"), "{said}");
    }
}

#[test]
fn never_leaves_a_numeric_weekday_in_what_it_returns() {
    for expression in [
        "0 9 * * 5",
        "0 9 * * 0-7",
        "0 9 * * 1-5/2",
        "0 9 * * */3",
        "0 9 * * 6-1",
        "0 9 * * 1,MON-3",
        "0 0 9 * * 5",
    ] {
        let named = named_weekdays(expression).unwrap();
        let weekday = named.split_whitespace().last().unwrap();
        assert!(
            !weekday.chars().any(|c| c.is_ascii_digit()),
            "{expression} -> {named}"
        );
        assert!(
            teeny_cadence::calendar::is_cron_expression(&named),
            "{named}"
        );
    }
    assert_eq!(named_weekdays("0 9 * * 0-7").unwrap(), "0 9 * * SUN-SAT");
}
