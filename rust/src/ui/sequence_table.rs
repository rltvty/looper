//! Sequence table UI component.
//!
//! Renders a scrollable table of sequence slots (A-Z) with columns for
//! loop name, length, repeat count, and next slot.

use iced::widget::{button, column, container, pick_list, row, scrollable, text, Column};
use iced::{Background, Border, Color, Element, Length, Theme};
use std::fmt;

use crate::playback::{PlaybackState, SequenceGrid, SlotId};

/// Column widths for consistent table layout.
const COL_ID_WIDTH: f32 = 40.0;
const COL_LOOP_WIDTH: f32 = 250.0;
const COL_LEN_WIDTH: f32 = 50.0;
const COL_QUAN_WIDTH: f32 = 60.0;
const COL_NEXT_WIDTH: f32 = 80.0;
const ROW_HEIGHT: f32 = 36.0;

/// Wrapper for next slot options in pick_list.
/// Represents either "None" (stop) or a specific slot ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextSlotOption(pub Option<SlotId>);

impl fmt::Display for NextSlotOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(id) => write!(f, "{}", id),
            None => write!(f, "--"),
        }
    }
}

impl NextSlotOption {
    /// All options: None followed by A-Z
    pub fn all_options() -> Vec<NextSlotOption> {
        let mut opts = vec![NextSlotOption(None)];
        for c in 'A'..='Z' {
            opts.push(NextSlotOption(Some(SlotId(c))));
        }
        opts
    }
}

/// Wrapper for loop options in pick_list.
/// Represents either "None" (empty slot) or an index into available_loops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopOption {
    pub index: Option<usize>,
    pub name: String,
}

impl fmt::Display for LoopOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl LoopOption {
    /// Build options list from available loops
    pub fn from_available(available: &[(String, Option<std::path::PathBuf>)]) -> Vec<LoopOption> {
        let mut opts = vec![LoopOption {
            index: None,
            name: "--".to_string(),
        }];
        for (i, (name, _)) in available.iter().enumerate() {
            // Truncate long names
            let display_name = if name.len() > 35 {
                format!("{}...", &name[..32])
            } else {
                name.clone()
            };
            opts.push(LoopOption {
                index: Some(i),
                name: display_name,
            });
        }
        opts
    }

    /// Find the option matching the current loop name
    pub fn find_selected(options: &[LoopOption], current_name: &str) -> Option<LoopOption> {
        if current_name == "--" {
            return options.first().cloned();
        }
        options.iter().find(|o| {
            o.name == current_name || current_name.starts_with(&o.name.trim_end_matches("..."))
        }).cloned()
    }
}

/// Row highlighting colors.
fn row_background(is_playing: bool, is_next: bool) -> Color {
    if is_playing {
        Color::from_rgb(0.15, 0.45, 0.15) // Green
    } else if is_next {
        Color::from_rgb(0.5, 0.35, 0.1) // Orange
    } else {
        Color::from_rgb(0.12, 0.12, 0.12) // Dark grey
    }
}

/// Header text color.
fn header_color() -> Color {
    Color::from_rgb(0.7, 0.7, 0.7)
}

/// Cell text color.
fn cell_color() -> Color {
    Color::from_rgb(0.9, 0.9, 0.9)
}

/// Render the table header row.
fn view_table_header<'a, M: 'a>() -> Element<'a, M> {
    let hdr_color = header_color();

    container(
        row![
            container(text("ID").size(12).color(hdr_color))
                .width(Length::Fixed(COL_ID_WIDTH))
                .padding([4, 8])
                .center_y(Length::Fixed(ROW_HEIGHT)),
            container(text("LOOP").size(12).color(hdr_color))
                .width(Length::Fixed(COL_LOOP_WIDTH))
                .padding([4, 8])
                .center_y(Length::Fixed(ROW_HEIGHT)),
            container(text("LEN").size(12).color(hdr_color))
                .width(Length::Fixed(COL_LEN_WIDTH))
                .padding([4, 8])
                .center_y(Length::Fixed(ROW_HEIGHT)),
            container(text("QUAN").size(12).color(hdr_color))
                .width(Length::Fixed(COL_QUAN_WIDTH))
                .padding([4, 8])
                .center_y(Length::Fixed(ROW_HEIGHT)),
            container(text("NEXT").size(12).color(hdr_color))
                .width(Length::Fixed(COL_NEXT_WIDTH))
                .padding([4, 8])
                .center_y(Length::Fixed(ROW_HEIGHT)),
        ]
        .spacing(2),
    )
    .style(|_theme: &Theme| container::Style {
        background: Some(Background::Color(Color::from_rgb(0.08, 0.08, 0.08))),
        border: Border::default().rounded(2),
        ..Default::default()
    })
    .into()
}


/// Render a single table row for a slot.
fn view_slot_row<'a, M: 'a + Clone>(
    slot_id: SlotId,
    loop_name: String,
    length_bars: String,
    repeat_count: u32,
    next_slot: Option<SlotId>,
    is_playing: bool,
    is_next: bool,
    loop_options: Vec<LoopOption>,
    on_loop_change: impl Fn(SlotId, LoopOption) -> M + 'a,
    on_next_change: impl Fn(SlotId, NextSlotOption) -> M + 'a,
    on_quan_decrement: M,
    on_quan_increment: M,
) -> Element<'a, M> {
    let bg_color = row_background(is_playing, is_next);
    let txt_color = cell_color();

    // Create pick_list for LOOP column
    let selected_loop = LoopOption::find_selected(&loop_options, &loop_name);
    let loop_picker = pick_list(loop_options, selected_loop, move |opt| {
        on_loop_change(slot_id, opt)
    })
    .text_size(11)
    .width(Length::Fixed(COL_LOOP_WIDTH - 8.0));

    // Create pick_list for NEXT column
    let next_options = NextSlotOption::all_options();
    let selected = NextSlotOption(next_slot);
    let next_picker = pick_list(next_options, Some(selected), move |opt| {
        on_next_change(slot_id, opt)
    })
    .text_size(12)
    .width(Length::Fixed(COL_NEXT_WIDTH - 8.0));

    // QUAN cell: - [count] + buttons
    let minus_btn = button(text("-").size(12).color(txt_color))
        .on_press(on_quan_decrement)
        .padding([1, 4])
        .style(move |_theme, _status| button::Style {
            background: Some(Background::Color(Color::from_rgb(0.25, 0.25, 0.25))),
            text_color: txt_color,
            border: Border::default().rounded(2),
            ..Default::default()
        });

    let plus_btn = button(text("+").size(12).color(txt_color))
        .on_press(on_quan_increment)
        .padding([1, 4])
        .style(move |_theme, _status| button::Style {
            background: Some(Background::Color(Color::from_rgb(0.25, 0.25, 0.25))),
            text_color: txt_color,
            border: Border::default().rounded(2),
            ..Default::default()
        });

    let quan_cell: Element<'a, M> = row![
        minus_btn,
        text(format!("{}", repeat_count)).size(12).color(txt_color),
        plus_btn,
    ]
    .spacing(2)
    .align_y(iced::Center)
    .into();

    container(
        row![
            container(text(slot_id.to_string()).size(14).color(txt_color))
                .width(Length::Fixed(COL_ID_WIDTH))
                .padding([4, 8])
                .center_y(Length::Fixed(ROW_HEIGHT)),
            container(loop_picker)
                .width(Length::Fixed(COL_LOOP_WIDTH))
                .padding([2, 4])
                .center_y(Length::Fixed(ROW_HEIGHT)),
            container(text(length_bars).size(14).color(txt_color))
                .width(Length::Fixed(COL_LEN_WIDTH))
                .padding([4, 8])
                .center_y(Length::Fixed(ROW_HEIGHT)),
            container(quan_cell)
                .width(Length::Fixed(COL_QUAN_WIDTH))
                .padding([2, 4])
                .center_y(Length::Fixed(ROW_HEIGHT)),
            container(next_picker)
                .width(Length::Fixed(COL_NEXT_WIDTH))
                .padding([2, 4])
                .center_y(Length::Fixed(ROW_HEIGHT)),
        ]
        .spacing(2),
    )
    .style(move |_theme: &Theme| container::Style {
        background: Some(Background::Color(bg_color)),
        border: Border::default().rounded(2),
        ..Default::default()
    })
    .height(Length::Fixed(ROW_HEIGHT))
    .into()
}

/// Build the complete scrollable sequence table.
///
/// Returns an Element that displays all 26 slots with highlighting for
/// the currently playing slot and the next slot.
///
/// Callbacks:
/// - `on_loop_change`: invoked when user changes a slot's loop
/// - `on_next_change`: invoked when user changes a slot's NEXT pointer
/// - `on_quan_decrement`: invoked when user clicks - to decrease repeat count
/// - `on_quan_increment`: invoked when user clicks + to increase repeat count
pub fn view_sequence_table<'a, M: 'a + Clone>(
    grid: &SequenceGrid,
    playback_state: Option<PlaybackState>,
    available_loops: &[(String, Option<std::path::PathBuf>)],
    on_loop_change: impl Fn(SlotId, Option<usize>) -> M + 'a + Copy,
    on_next_change: impl Fn(SlotId, Option<SlotId>) -> M + 'a + Copy,
    on_quan_decrement: impl Fn(SlotId) -> M + 'a + Copy,
    on_quan_increment: impl Fn(SlotId) -> M + 'a + Copy,
) -> Element<'a, M> {
    let current_slot = playback_state.map(|s| s.current_slot);
    let next_slot = playback_state.and_then(|s| grid.get(s.current_slot).next_slot);

    // Build loop options once
    let loop_options = LoopOption::from_available(available_loops);

    // Build all rows
    let rows: Vec<Element<'_, M>> = grid
        .slots
        .iter()
        .map(|slot| {
            let is_playing = current_slot == Some(slot.id);
            let is_next = next_slot == Some(slot.id);

            view_slot_row(
                slot.id,
                slot.loop_name().to_string(),
                slot.length_bars(),
                slot.repeat_count,
                slot.next_slot,
                is_playing,
                is_next,
                loop_options.clone(),
                move |slot_id, opt| on_loop_change(slot_id, opt.index),
                move |slot_id, opt| on_next_change(slot_id, opt.0),
                on_quan_decrement(slot.id),
                on_quan_increment(slot.id),
            )
        })
        .collect();

    let table_content = column![view_table_header(), Column::with_children(rows).spacing(2),]
        .spacing(4)
        .padding(8);

    // Wrap in scrollable - show ~8 rows at a time
    scrollable(table_content)
        .height(Length::Fixed(340.0))
        .into()
}
