//! Non-modal project navigation and background work, using shared dock chrome.
use crate::{
    app::Message,
    ui::dock::{DockMsg, PanelId},
};
use iced::{
    widget::{
        button, column, container, mouse_area, progress_bar, row, scrollable, text, text_input,
        Space,
    },
    Element, Fill, Length,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Drawing(u64),
    Entity(u64, u64),
    Cloud(u64, String),
    Terrain(u64),
    Gis(u64, String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JobKind {
    Terrain,
    Basemap,
    Index,
    Export,
    Merge,
    Urban,
}
#[derive(Clone, Debug)]
pub struct CancelJob {
    pub tab: u64,
    pub kind: JobKind,
    pub source: String,
    pub token: u64,
}
#[derive(Clone, Debug)]
pub struct JobRow {
    pub name: String,
    pub detail: String,
    pub completed: u64,
    pub total: u64,
    pub cancel: Option<CancelJob>,
}
#[derive(Clone, Debug)]
pub struct ExplorerRow {
    pub group: &'static str,
    pub name: String,
    pub detail: String,
    pub target: Target,
}

fn header(id: PanelId, pinned: bool) -> Element<'static, Message> {
    let pin = button(if pinned { "Pin" } else { "Auto" })
        .padding([2, 5])
        .style(button::subtle)
        .on_press(Message::Dock(DockMsg::AutoCollapseToggle(id)));
    let close = button("Close")
        .padding([2, 5])
        .style(button::subtle)
        .on_press(Message::Dock(DockMsg::Close(id)));
    mouse_area(
        container(
            row![
                text(id.title()).size(13),
                Space::new().width(Fill),
                pin,
                close
            ]
            .spacing(3),
        )
        .padding([4, 6])
        .width(Fill),
    )
    .on_press(Message::Dock(DockMsg::DockGrab(id)))
    .into()
}

pub fn explorer<'a>(
    search: &'a str,
    rows: Vec<ExplorerRow>,
    width: f32,
    auto: bool,
) -> Element<'a, Message> {
    let query = search.to_lowercase();
    let mut body = column![].spacing(5);
    let mut group = "";
    let mut matched = false;
    for item in rows.into_iter().filter(|r| {
        query.is_empty()
            || format!("{} {} {}", r.group, r.name, r.detail)
                .to_lowercase()
                .contains(&query)
    }) {
        matched = true;
        if group != item.group {
            group = item.group;
            body = body.push(text(group).size(13));
        }
        body = body.push(
            button(column![text(item.name).size(12), text(item.detail).size(10)].spacing(2))
                .width(Fill)
                .padding([5, 7])
                .style(button::subtle)
                .on_press(Message::WorkspaceFocus(item.target)),
        );
    }
    if !matched {
        body = body.push(text("No matching items.").size(12));
    }
    let actions = row![
        button("Terrain").on_press(Message::Command("TERRAINPANEL".into())),
        button("Jobs").on_press(Message::Command("JOBSPANEL".into()))
    ]
    .spacing(4);
    container(
        column![
            header(PanelId::ProjectExplorer, auto),
            actions,
            text_input("Search project…", search)
                .on_input(Message::WorkspaceSearch)
                .size(12)
                .padding(6),
            text("Click an object to select and zoom. Up to 200 CAD objects are shown.").size(10),
            scrollable(body).height(Fill)
        ]
        .spacing(6)
        .padding(5),
    )
    .width(Length::Fixed(width))
    .height(Fill)
    .style(container::bordered_box)
    .into()
}

pub fn jobs(
    rows: Vec<JobRow>,
    history: Vec<String>,
    width: f32,
    auto: bool,
) -> Element<'static, Message> {
    let mut body = column![text("Active work in all drawings").size(12)].spacing(8);
    if rows.is_empty() {
        body = body.push(text("No background jobs running.").size(12));
    }
    for job in rows {
        let mut line = row![text(job.name).size(12), Space::new().width(Fill)].spacing(4);
        if let Some(cancel) = job.cancel {
            line = line.push(
                button("Cancel")
                    .padding([2, 5])
                    .on_press(Message::WorkspaceCancel(cancel)),
            );
        }
        let mut card = column![line, text(job.detail).size(10)].spacing(4);
        if job.total > 0 {
            let percent = ((job.completed.min(job.total) as f64 / job.total as f64) * 100.) as f32;
            card = card
                .push(progress_bar(0.0..=100.0, percent).girth(5))
                .push(text(format!("{} / {} ({percent:.0}%)", job.completed, job.total)).size(10));
        }
        body = body.push(
            container(card)
                .padding(7)
                .width(Fill)
                .style(container::bordered_box),
        );
    }
    body = body.push(text("Project job history").size(13));
    if history.is_empty() {
        body = body.push(text("No recorded jobs in the active spatial project.").size(11));
    }
    for item in history {
        body = body.push(text(item).size(11));
    }
    body = body.push(button("Open command log").on_press(Message::WorkspaceLog));
    container(
        column![header(PanelId::Jobs, auto), scrollable(body).height(Fill)]
            .spacing(5)
            .padding(5),
    )
    .width(Length::Fixed(width))
    .height(Fill)
    .style(container::bordered_box)
    .into()
}
