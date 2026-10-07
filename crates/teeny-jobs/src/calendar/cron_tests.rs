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
