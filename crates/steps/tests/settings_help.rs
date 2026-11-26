//! Every choice of every built-in step has words to show, and a
//! setting's help, where it has any, is one short sentence.

use trip_engine::{ParamKind, HELP_MAX};

#[test]
fn every_choice_has_a_label_and_help_is_short() {
    let (real, sim) = (trip_steps::real(), trip_steps::simulated(None));
    let mut checked = 0;
    for info in real.infos().chain(sim.infos()) {
        for p in &info.params {
            let at = format!("{} · {}", info.kind, p.key);
            if let Some(help) = p.help {
                assert!(!help.trim().is_empty(), "{at}'s help is empty");
                assert!(help.chars().count() <= HELP_MAX, "{at}'s help is {} characters, over {HELP_MAX}: {help}", help.chars().count());
            }
            match p.kind {
                ParamKind::Choice(values) => {
                    assert!(p.labels.is_empty() || p.labels.len() == values.len(), "{at}: {} labels for {} values", p.labels.len(), values.len());
                    for v in values {
                        assert!(!p.choice_label(v).trim().is_empty(), "{at}: {v} has no label");
                    }
                }
                _ => assert!(p.labels.is_empty(), "{at} isn't a choice but has labels"),
            }
            checked += 1;
        }
    }
    assert!(checked > 40, "only {checked} settings found");
}
