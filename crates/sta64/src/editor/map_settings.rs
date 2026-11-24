//! Map-level BZN metadata and the existing player start-location records.

use std::collections::BTreeMap;

use bevy::prelude::*;

use super::Document;
use crate::ui::theme::{ScrollArea, TextField, TextFilter, Theme, kit_text, text_field};

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Field {
    Description,
    Background,
    Starfield,
    Ambient(usize),
    StartCount,
    StartPos(usize, usize),
    StartKind(usize),
    StartTeam(usize),
    StartRace(usize),
}

struct Row {
    section: &'static str,
    label: String,
    field: Field,
    value: String,
}

fn rows(doc: &Document) -> Vec<Row> {
    let map = &doc.map;
    let mut rows = Vec::new();
    let mut add = |section, label: String, field, value: String| {
        rows.push(Row {
            section,
            label,
            field,
            value,
        });
    };
    add(
        "Map",
        "Description".into(),
        Field::Description,
        map.description.clone(),
    );
    add(
        "Map",
        "Skybox".into(),
        Field::Background,
        map.background.clone(),
    );
    add(
        "Map",
        "Starfield ODF".into(),
        Field::Starfield,
        map.starfield_odf.clone(),
    );
    for (index, (name, value)) in ["Red", "Green", "Blue"]
        .into_iter()
        .zip(map.ambient)
        .enumerate()
    {
        add(
            "Ambient light",
            name.into(),
            Field::Ambient(index),
            value.to_string(),
        );
    }
    add(
        "Start locations",
        "Playable count".into(),
        Field::StartCount,
        map.start_count.to_string(),
    );
    for (index, loc) in map.start_locs.iter().enumerate() {
        let section = "Start locations";
        let slot = format!("Slot {index}");
        for (axis, (name, value)) in ["X", "Y", "Z"].into_iter().zip(loc.pos).enumerate() {
            add(
                section,
                format!("{slot} {name}"),
                Field::StartPos(index, axis),
                value.to_string(),
            );
        }
        add(
            section,
            format!("{slot} type"),
            Field::StartKind(index),
            loc.kind.clone(),
        );
        add(
            section,
            format!("{slot} team"),
            Field::StartTeam(index),
            loc.team.to_string(),
        );
        add(
            section,
            format!("{slot} race"),
            Field::StartRace(index),
            loc.race.clone(),
        );
    }
    rows
}

/// Draw only the editable rows; the caller owns the dialog title and actions.
pub(super) fn draw(ui: &mut ChildSpawnerCommands, theme: &Theme, doc: &Document, width: f32) {
    const ROW_H: f32 = 40.0;
    let rows = rows(doc);
    let count = rows.len()
        + rows
            .iter()
            .enumerate()
            .filter(|(i, row)| *i == 0 || rows[*i - 1].section != row.section)
            .count();
    let visible = count.min(12);
    let mut field_theme = theme.clone();
    field_theme.button_h = field_theme.button_h.min(26.0);
    ui.spawn((
        ScrollArea {
            row_h: ROW_H,
            rows: visible,
            count,
        },
        ScrollPosition::default(),
        Node {
            width: Val::Px(width),
            height: Val::Px(ROW_H * visible as f32),
            flex_direction: FlexDirection::Column,
            flex_shrink: 0.0,
            overflow: Overflow::scroll_y(),
            ..default()
        },
        BackgroundColor(theme.track),
    ))
    .with_children(|list| {
        let mut section = None;
        for row in rows {
            if section != Some(row.section) {
                section = Some(row.section);
                list.spawn(Node {
                        height: Val::Px(ROW_H),
                        flex_shrink: 0.0,
                        padding: UiRect::horizontal(Val::Px(theme.pad)),
                        align_items: AlignItems::Center,
                        ..default()
                    }).with_child(kit_text(theme, row.section, theme.text_size, theme.title));
            }
            list.spawn((Node {
                height: Val::Px(ROW_H),
                flex_shrink: 0.0,
                padding: UiRect::horizontal(Val::Px(theme.pad)),
                align_items: AlignItems::Center,
                column_gap: Val::Px(4.0),
                ..default()
            },))
                .with_children(|line| {
                    line.spawn(Node {
                            width: Val::Px(116.0),
                            flex_shrink: 0.0,
                            overflow: Overflow::clip(),
                            ..default()
                        }).with_child(kit_text(theme, &row.label, theme.small_size, theme.text_dim));
                    line.spawn((
                        text_field(
                            &field_theme,
                            &row.value,
                            "",
                            width - 136.0,
                            2048.max(row.value.chars().count()),
                            TextFilter::Any,
                        ),
                        row.field,
                    ));
                });
        }
    });
}

fn text(value: &str, label: &str) -> Result<String, String> {
    if value.contains(['\0', '\r', '\n']) {
        return Err(format!("{label} cannot contain line breaks or NUL"));
    }
    Ok(value.to_owned())
}

fn number(value: &str, label: &str) -> Result<f32, String> {
    value
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("Enter a finite number for {label}"))
}

fn integer(value: &str, label: &str) -> Result<i32, String> {
    value
        .trim()
        .parse()
        .map_err(|_| format!("Enter an integer for {label}"))
}

/// Commit all edited fields together, after the document's own save path has
/// proved the proposed metadata survives writing and reparsing.
pub(super) fn apply(
    document: &mut Document,
    fields: &Query<(&Field, &TextField)>,
) -> Result<(), String> {
    if !document.can_change_objects() {
        return Err("Map settings cannot be changed for this map format".into());
    }
    let expected = rows(document);
    let mut inputs = BTreeMap::new();
    for (field, input) in fields.iter() {
        if inputs.insert(*field, input.text.as_str()).is_some() {
            return Err("Map settings contains a duplicate field".into());
        }
    }
    if inputs.len() != expected.len() || expected.iter().any(|row| !inputs.contains_key(&row.field))
    {
        return Err("Map settings form is incomplete".into());
    }
    let mut proposed = document.clone();
    for row in expected {
        let value = inputs[&row.field];
        // Preserve values in older or modded maps that the current form does
        // not author, provided the user leaves those fields untouched.
        if value == row.value {
            continue;
        }
        match row.field {
            Field::Description => proposed.map.description = text(value, "Description")?,
            Field::Background => proposed.map.background = text(value, "Skybox")?,
            Field::Starfield => proposed.map.starfield_odf = text(value, "Starfield ODF")?,
            Field::Ambient(axis) => proposed.map.ambient[axis] = number(value, &row.label)?,
            Field::StartCount => {
                let count = integer(value, "Playable count")?;
                if !(0..=8).contains(&count) {
                    return Err("Playable count must be 0 through 8".into());
                }
                proposed.map.start_count = count;
            }
            Field::StartPos(slot, axis) => {
                proposed.map.start_locs[slot].pos[axis] = number(value, &row.label)?
            }
            Field::StartKind(slot) => {
                let kind = text(value, &row.label)?;
                if !matches!(
                    kind.as_str(),
                    "" | "player" | "ai-easy" | "ai-medium" | "ai-hard" | "empty"
                ) {
                    return Err(format!(
                        "{} must be player, ai-easy, ai-medium, ai-hard, empty, or blank",
                        row.label
                    ));
                }
                proposed.map.start_locs[slot].kind = kind;
            }
            Field::StartTeam(slot) => {
                let team = integer(value, &row.label)?;
                if !(-1..=8).contains(&team) {
                    return Err(format!("{} must be -1 through 8", row.label));
                }
                proposed.map.start_locs[slot].team = team;
            }
            Field::StartRace(slot) => proposed.map.start_locs[slot].race = text(value, &row.label)?,
        }
    }
    let bytes = proposed.bytes()?;
    let parsed = sta_bzn::parse(&bytes).map_err(|error| error.to_string())?;
    let wanted = &proposed.map;
    if parsed.description != wanted.description
        || parsed.background != wanted.background
        || parsed.starfield_odf != wanted.starfield_odf
        || parsed.ambient != wanted.ambient
        || parsed.start_count != wanted.start_count
        || parsed.start_locs != wanted.start_locs
    {
        return Err("Map settings did not round trip through the BZN writer".into());
    }
    document.map = proposed.map;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sta_bzn::{Map, StartLoc};

    #[derive(Resource)]
    struct Outcome(Result<(), String>);

    fn commit(
        mut document: ResMut<Document>,
        fields: Query<(&Field, &TextField)>,
        mut outcome: ResMut<Outcome>,
    ) {
        outcome.0 = apply(&mut document, &fields);
    }

    fn app() -> App {
        let map = Map {
            version: 3013,
            description: "Original".into(),
            background: "mbg1".into(),
            starfield_odf: "stars.odf".into(),
            ambient: [0.2, 0.3, 0.4],
            start_count: 1,
            start_locs: vec![StartLoc {
                pos: [1.0, 2.0, 3.0],
                kind: "player".into(),
                team: -1,
                race: "race-unspecified".into(),
            }],
            ..default()
        };
        let bytes = sta_bzn::write::write(&map).unwrap();
        let document = Document::new("fixture.bzn".into(), bytes, map).unwrap();
        let mut app = App::new();
        for row in rows(&document) {
            app.world_mut().spawn((
                row.field,
                TextField {
                    text: row.value.clone(),
                    cursor: row.value.chars().count(),
                    max_len: 2048,
                    filter: TextFilter::Any,
                    placeholder: String::new(),
                },
            ));
        }
        app.insert_resource(document)
            .insert_resource(Outcome(Err("not run".into())))
            .add_systems(Update, commit);
        app
    }

    fn edit(app: &mut App, field: Field, value: &str) {
        let entities: Vec<_> = app
            .world_mut()
            .iter_entities()
            .filter(|entity| entity.get::<Field>() == Some(&field))
            .map(|entity| entity.id())
            .collect();
        assert_eq!(entities.len(), 1);
        app.world_mut()
            .entity_mut(entities[0])
            .get_mut::<TextField>()
            .unwrap()
            .text = value.into();
    }

    #[test]
    fn settings_apply_and_round_trip_to_bzn() {
        let mut app = app();
        edit(&mut app, Field::Description, "Edited map");
        edit(&mut app, Field::Background, "mbg2");
        edit(&mut app, Field::Ambient(1), "0.75");
        edit(&mut app, Field::StartPos(0, 0), "42");
        edit(&mut app, Field::StartRace(0), "federation");
        app.update();
        assert!(app.world().resource::<Outcome>().0.is_ok());
        let document = app.world().resource::<Document>();
        let parsed = sta_bzn::parse(&document.bytes().unwrap()).unwrap();
        assert_eq!(parsed.description, "Edited map");
        assert_eq!(parsed.background, "mbg2");
        assert_eq!(parsed.ambient[1], 0.75);
        assert_eq!(parsed.start_locs[0].pos[0], 42.0);
        assert_eq!(parsed.start_locs[0].race, "federation");
    }

    #[test]
    fn invalid_field_does_not_commit_other_edits() {
        let mut app = app();
        edit(&mut app, Field::Description, "Should not save");
        edit(&mut app, Field::Ambient(1), "not a number");
        app.update();
        assert!(app.world().resource::<Outcome>().0.is_err());
        assert_eq!(
            app.world().resource::<Document>().map.description,
            "Original"
        );
    }
}
