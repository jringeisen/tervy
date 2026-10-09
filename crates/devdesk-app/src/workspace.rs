//! The main window layout: project sidebar on the left, workspace on the right.
//!
//! M0 renders placeholders only. Projects arrive in M2 and terminals in M1.

use gpui_kit::base::StyledExt as _;
use gpui_kit::component::sidebar::{Sidebar, SidebarGroup, SidebarHeader, SidebarMenu};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::*;

pub struct Workspace;

impl Workspace {
    pub fn new() -> Self {
        Self
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        h_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                Sidebar::new("projects-sidebar")
                    .w(px(240.))
                    .header(SidebarHeader::new().child(div().font_bold().child("DevDesk")))
                    .child(SidebarGroup::new("Projects").child(SidebarMenu::new())),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(div().text_lg().child("No project open"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("Terminals arrive in M1, projects in M2."),
                    ),
            )
    }
}
