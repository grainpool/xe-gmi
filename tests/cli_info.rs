mod common;
use common::*;

#[test]
fn info_b65_golden() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["info"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "info_b65.txt");
}

#[test]
fn info_section_power_golden() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["info", "--section", "power"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_golden(&stdout(&o), "info_b65_power.txt");
}

#[test]
fn info_verbose_explains_na() {
    let o = Runner::fixture("b65-g31-k7.1").run(&["-v", "info", "--section", "power"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    // PL1 is N/A on the B65; verbose must say which file was looked for.
    assert_contains(&s, "power1_max");
}

#[test]
fn info_dg2_shows_pl1_and_rated_max_and_voltage() {
    let o = Runner::fixture("dg2-a770-k6.12-forced").run(&["info", "--section", "power"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    assert_contains(&s, "190.00 W (pl1 card)");
    assert_contains(&s, "Rated max card");
    assert_contains(&s, "Voltage pkg");
    assert_contains(&s, "1120 mV");
}

#[test]
fn info_multi_tile_lists_four_gts() {
    let o = Runner::fixture("multi-tile-2x2-k7.1").run(&["info", "--section", "clocks"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let s = stdout(&o);
    for gt in [
        "GT 0 (tile 0, gt0-rc)",
        "GT 1 (tile 0, gt1-mc)",
        "GT 2 (tile 1, gt2-rc)",
        "GT 3 (tile 1, gt3-mc)",
    ] {
        assert_contains(&s, gt);
    }
}
