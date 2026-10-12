use super::*;

#[test]
fn recall_names_the_person_then_the_general_context() {
    let fren = Person::x("7", Some("fren"));
    assert_eq!(
        format_recall(
            Some(&fren),
            "- asked about the chart",
            "- the chart is hourly"
        ),
        "ABOUT @fren:\n- asked about the chart\n\nGENERAL CONTEXT:\n- the chart is hourly"
    );
    let nameless = Person::x("7", None);
    assert_eq!(
        format_recall(Some(&nameless), "- quiet one", "  "),
        "ABOUT this person:\n- quiet one"
    );
    assert_eq!(
        format_recall(None, "", "- facts"),
        "GENERAL CONTEXT:\n- facts"
    );
    assert_eq!(format_recall(Some(&fren), " \n", ""), "");
}
