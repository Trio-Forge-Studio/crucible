use crucible_iac::model::Kind;
use crucible_iac::oc::scopes;
use crucible_iac::provision::Write;

fn names(reads: &[Kind], writes: &[Write]) -> Vec<String> {
    scopes(reads, writes).into_iter().map(|scope| scope.name).collect()
}

#[test]
fn reads_need_the_read_scopes_of_their_kinds_and_badges_need_none() {
    assert_eq!(names(&[Kind::Product, Kind::Pass], &[]), ["developer-product:read", "game-pass:read"]);
    assert_eq!(names(&[Kind::Badge], &[]), Vec::<String>::new(), "the public badge list needs no key");
}

#[test]
fn each_write_needs_the_scopes_its_operation_declares() {
    assert_eq!(names(&[], &[Write::Create(Kind::Product)]), ["developer-product:write"]);
    assert_eq!(names(&[], &[Write::Create(Kind::Badge)]), ["legacy-universe.badge:manage-and-spend-robux"]);
    let retire = Write::Update { kind: Kind::Pass, details: true, icon: false };
    assert_eq!(names(&[], &[retire]), ["game-pass:write"]);
    let badge = |details, icon| Write::Update { kind: Kind::Badge, details, icon };
    assert_eq!(names(&[], &[badge(false, true)]), ["legacy-badge:manage"], "only an icon change needs it");
    assert_eq!(
        names(&[], &[badge(true, false)]),
        ["legacy-universe.badge:manage-and-spend-robux", "legacy-universe.badge:write"]
    );
    assert_eq!(names(&[], &[Write::Settings]), ["universe:write"]);
}

#[test]
fn a_scope_two_calls_need_is_named_once() {
    let writes = [Write::Create(Kind::Product), Write::Update { kind: Kind::Product, details: true, icon: true }];
    assert_eq!(names(&[Kind::Product], &writes), ["developer-product:read", "developer-product:write"]);
}
