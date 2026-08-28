//! Visual workflow builder and company-standards package manager.

use crate::app::Message;
use iced::widget::{button, column, container, row, scrollable, text, text_editor, Space};
use iced::{Background, Border, Element, Length, Theme};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlatformManagerTab {
    #[default]
    Workflows,
    Standards,
}

pub struct PlatformManagerState {
    pub tab: PlatformManagerTab,
    pub workflow: text_editor::Content,
    pub standards: text_editor::Content,
    pub status: String,
}

impl Default for PlatformManagerState {
    fn default() -> Self {
        Self {
            tab: PlatformManagerTab::Workflows,
            workflow: text_editor::Content::with_text(&workflow_template()),
            standards: text_editor::Content::with_text(&standards_template()),
            status: "Create or open a spatial project, then apply a definition.".into(),
        }
    }
}

impl PlatformManagerState {
    pub fn load(&mut self, platform: Option<&ocs_platform::PlatformState>) {
        if let Some(workflow) = platform.and_then(|value| value.workflows.first()) {
            self.workflow = text_editor::Content::with_text(
                &serde_json::to_string_pretty(workflow).unwrap_or_else(|_| workflow_template()),
            );
        }
        if let Some(standards) = platform.and_then(|value| value.standards.first()) {
            self.standards = text_editor::Content::with_text(
                &serde_json::to_string_pretty(standards).unwrap_or_else(|_| standards_template()),
            );
        }
        let (workflows, standards) = platform
            .map(|value| (value.workflows.len(), value.standards.len()))
            .unwrap_or_default();
        self.status = format!("{workflows} workflow(s), {standards} standards package(s)");
    }

    pub fn reset_workflow(&mut self) {
        self.workflow = text_editor::Content::with_text(&workflow_template());
        self.status = "New workflow template ready.".into();
    }

    pub fn reset_standards(&mut self) {
        self.standards = text_editor::Content::with_text(&standards_template());
        self.status = "New standards template ready; Apply seals its SHA-256 digest.".into();
    }
}

fn workflow_template() -> String {
    serde_json::to_string_pretty(&ocs_platform::WorkflowDefinition {
        id: "production-lidar".into(),
        name: "Production LiDAR".into(),
        api_version: 1,
        nodes: vec![
            ocs_platform::WorkflowNode {
                id: "classify".into(),
                tool_id: "lidar.classify.pipeline".into(),
                parameters: serde_json::json!({}),
                bindings: Default::default(),
            },
            ocs_platform::WorkflowNode {
                id: "surface".into(),
                tool_id: "lidar.surface.dtm".into(),
                parameters: serde_json::json!({"cell_size": 1.0}),
                bindings: Default::default(),
            },
        ],
        edges: vec![ocs_platform::WorkflowEdge {
            from: "classify".into(),
            to: "surface".into(),
        }],
        metadata: Default::default(),
    })
    .expect("workflow template serializes")
}

fn standards_template() -> String {
    let mut package = ocs_platform::StandardsPackage {
        id: "company-survey".into(),
        name: "Company Survey Standards".into(),
        version: "1.0.0".into(),
        tool_presets: std::collections::BTreeMap::from([(
            "lidar.surface.dtm".into(),
            serde_json::json!({"cell_size": 1.0}),
        )]),
        validation_rules: vec![ocs_platform::ValidationRule {
            id: "crs-required".into(),
            description: "A declared horizontal CRS is required".into(),
            severity: ocs_platform::ValidationSeverity::Error,
            target_kind: "spatial_project".into(),
            expression: serde_json::json!({"field": "spatial_reference.horizontal", "present": true}),
        }],
        content_sha256: String::new(),
        signer: None,
        signature: None,
    };
    package.seal().expect("standards template seals");
    serde_json::to_string_pretty(&package).expect("standards template serializes")
}

fn card<'a>(title: String, subtitle: String) -> Element<'a, Message> {
    container(column![text(title).size(13), text(subtitle).size(10)].spacing(3))
        .padding(9)
        .width(Length::Fill)
        .style(|theme: &Theme| container::Style {
            background: Some(Background::Color(theme.palette().background.weak.color)),
            border: Border {
                color: theme.palette().primary.weak.color,
                width: 1.0,
                radius: 5.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn workflow_preview<'a>(json: &str) -> Element<'a, Message> {
    match serde_json::from_str::<ocs_platform::WorkflowDefinition>(json) {
        Ok(workflow) => {
            let mut graph = column![text(format!(
                "{} · {} node(s) · {} connection(s)",
                workflow.name,
                workflow.nodes.len(),
                workflow.edges.len()
            ))
            .size(13)]
            .spacing(5);
            for (index, node) in workflow.nodes.iter().enumerate() {
                if index > 0 {
                    graph = graph.push(text("↓ dependency / execution order").size(10));
                }
                graph = graph.push(card(node.id.clone(), node.tool_id.clone()));
            }
            scrollable(graph).height(Length::Fill).into()
        }
        Err(error) => text(format!("Graph preview unavailable: {error}")).size(11).into(),
    }
}

fn standards_preview<'a>(json: &str) -> Element<'a, Message> {
    match serde_json::from_str::<ocs_platform::StandardsPackage>(json) {
        Ok(package) => {
            let mut rules = column![
                text(format!("{} · v{}", package.name, package.version)).size(13),
                text(format!(
                    "{} preset(s) · {} validation rule(s)",
                    package.tool_presets.len(),
                    package.validation_rules.len()
                ))
                .size(11),
            ]
            .spacing(6);
            for rule in package.validation_rules {
                rules = rules.push(card(rule.id, format!("{:?} · {}", rule.severity, rule.description)));
            }
            scrollable(rules).height(Length::Fill).into()
        }
        Err(error) => text(format!("Standards preview unavailable: {error}")).size(11).into(),
    }
}

pub fn view_window<'a>(
    state: &'a PlatformManagerState,
    has_project: bool,
    sizing: crate::ui::modal::ModalSizing,
) -> Element<'a, Message> {
    let workflows = button(text("Workflow Builder").size(12))
        .on_press(Message::PlatformManagerTab(PlatformManagerTab::Workflows));
    let standards = button(text("Company Standards").size(12))
        .on_press(Message::PlatformManagerTab(PlatformManagerTab::Standards));
    let project = if has_project {
        "Spatial project connected"
    } else {
        "No spatial project — create/open one before Apply"
    };

    let (preview, editor, actions): (Element<'a, Message>, Element<'a, Message>, Element<'a, Message>) =
        match state.tab {
            PlatformManagerTab::Workflows => (
                workflow_preview(&state.workflow.text()),
                text_editor(&state.workflow)
                    .on_action(Message::PlatformWorkflowEdit)
                    .size(11)
                    .height(Length::Fill)
                    .into(),
                row![
                    button("New").on_press(Message::PlatformWorkflowNew),
                    button("Validate + Apply").on_press(Message::PlatformWorkflowApply),
                    button("Delete").on_press(Message::PlatformWorkflowDelete),
                ]
                .spacing(7)
                .into(),
            ),
            PlatformManagerTab::Standards => (
                standards_preview(&state.standards.text()),
                text_editor(&state.standards)
                    .on_action(Message::PlatformStandardsEdit)
                    .size(11)
                    .height(Length::Fill)
                    .into(),
                row![
                    button("New").on_press(Message::PlatformStandardsNew),
                    button("Import").on_press(Message::PlatformStandardsImport),
                    button("Export").on_press(Message::PlatformStandardsExport),
                    button("Validate + Apply").on_press(Message::PlatformStandardsApply),
                    button("Trust Signer").on_press(Message::PlatformStandardsTrustSigner),
                    button("Delete").on_press(Message::PlatformStandardsDelete),
                ]
                .spacing(7)
                .into(),
            ),
        };

    let body = row![
        container(preview).width(Length::FillPortion(2)).height(Length::Fill),
        container(editor).width(Length::FillPortion(3)).height(Length::Fill),
    ]
    .spacing(12)
    .height(Length::Fill);

    container(
        column![
            row![workflows, standards, Space::new().width(Length::Fill), text(project).size(11)]
                .spacing(7),
            body,
            actions,
            text(&state.status).size(11),
        ]
        .spacing(10)
        .padding(14),
    )
    .width(sizing.width)
    .height(sizing.height)
    .into()
}
