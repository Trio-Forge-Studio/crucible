mod support;

use crucible_compiler::{Compiled, Failure, Target};
use crucible_transform::{CompileInfo, IconAtlas, IconMode, IconRect};
use support::{Game, has};

fn compile(game: &Game, icons: IconMode) -> Result<(Compiled, CompileInfo), Failure> {
    let instances = game.instances().unwrap();
    let include = game.include();
    crucible_transform::compile_game(
        &game.dir.join("tsconfig.json"),
        Target::Game { instances: &instances, include: &include, modules: &[] },
        icons,
    )
}

#[test]
fn only_the_imported_icon_requires_a_literal() {
    let game = Game::new();
    game.write(
        "src/client/icons.ts",
        r#"
            import { icon as glyph } from "@trioforge/crucible/ui";
            const name = "house";
            export const image = glyph(name);
            function icon(value: string) { return value; }
            export const plain = icon(name);
        "#,
    );
    let Err(Failure::Crucible(diagnostics)) = compile(&game, IconMode::Collect) else {
        panic!("a variable passed to the macro must fail");
    };
    let rule = diagnostics.iter().filter(|d| d.code >= 90000).collect::<Vec<_>>();
    assert_eq!(rule.len(), 1, "{diagnostics:?}");
    assert_eq!(rule[0].code, 90041);
    assert!(rule[0].message.contains("bakes only the icons it can see"));
}

#[test]
fn icon_collection_deduplicates_without_an_atlas_and_build_reports_missing_entries() {
    let game = Game::new();
    game.write(
        "src/client/icons.ts",
        r#"
            import * as ui from "@trioforge/crucible/ui";
            export const a = ui.icon("settings");
            export const b = ui.icon("house");
            export const c = ui.icon("settings");
        "#,
    );
    let (_, info) = compile(&game, IconMode::Collect).unwrap();
    assert_eq!(info.icons, ["house", "settings"]);
    let Err(Failure::Crucible(diagnostics)) = compile(&game, IconMode::default()) else {
        panic!("unlocked icons must fail a build");
    };
    let missing: Vec<_> = diagnostics.iter().filter(|d| d.code == 90042).collect();
    assert_eq!(missing.len(), 3, "{diagnostics:?}");
    assert!(missing.iter().all(|d| d.message.contains("`crucible icons`")));
    assert!(missing[1].message.contains("house"));
}

#[test]
fn icon_with_an_atlas_emits_a_literal_image_table() {
    let game = Game::new();
    game.write(
        "src/client/icons.ts",
        r#"
            import { icon as glyph } from "@trioforge/crucible/ui";
            export const image = glyph("house");
        "#,
    );
    let mode =
        IconMode::Build(Ok(IconAtlas::from([("house".into(), IconRect { asset: "847".into(), x: 66, y: 130 })])));
    let (compiled, info) = compile(&game, mode).unwrap();
    assert_eq!(info.icons, ["house"]);
    let source = &compiled.modules.iter().find(|m| m.path == "client/icons.luau").unwrap().source;
    has(source, "Image = \"rbxassetid://847\"");
    has(source, "ImageRectOffset = Vector2.new(66, 130)");
    has(source, "ImageRectSize = Vector2.new(60, 60)");
    assert!(!source.contains("glyph(\"house\")"), "{source}");
}
