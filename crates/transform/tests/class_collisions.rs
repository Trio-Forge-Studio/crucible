use crucible_compiler::{Failure, Target};
use std::path::PathBuf;

#[test]
fn methods_cannot_collide_with_opposite_class_properties() {
    let config = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/class-collisions/tsconfig.json");
    let Err(Failure::Crucible(errors)) = crucible_transform::compile(&config, Target::Package) else {
        panic!("static and instance collisions must fail compilation");
    };
    let actual: Vec<_> = errors.iter().map(|e| e.message.as_str()).collect();
    let instance = "instance methods cannot use the same name as static properties";
    let static_ = "static methods cannot use the same name as instance properties";
    assert_eq!(
        actual,
        [
            static_, instance, instance, static_, instance, static_, instance, static_, instance, static_, instance,
            static_, instance, static_, instance, static_, instance
        ]
    );
    assert!(errors.iter().all(|e| e.line > 1 && e.column > 1 && e.code == 0));
}
