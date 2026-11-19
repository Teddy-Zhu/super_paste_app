use gpui_kit::{prelude::*, *};
use super_paste_format::MCP_TOOLS;
use super::{McpPane, code_state, copy, guide, render::{code_block, fold, note}};
use crate::ui::palette::Palette;

impl McpPane {
    pub(super) fn reference(&mut self, p: Palette, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let weak = cx.entity().downgrade(); let toggle = weak.clone();
        let heading = fold("mcp-reference", "工具与使用说明", self.reference_open, p, window, cx, move |_, cx| { let _ = toggle.update(cx, |this, cx| { this.reference_open = !this.reference_open; if !this.reference_open { this.push_code = None; } cx.notify(); }); })
            .py(px(16.0)).text_size(px(13.0)).font_weight(FontWeight::MEDIUM)
            .child(div().ml_auto().text_size(px(11.0)).font_weight(FontWeight::NORMAL).text_color(p.text_tertiary).child("8 项工具"));
        let mut root = div().mt(px(24.0)).border_t_1().border_color(p.border_medium).child(heading);
        if !self.reference_open { return root; }
        let tools = MCP_TOOLS.into_iter().enumerate().map(|(index, tool)| {
            let expanded = self.tools_open & (1 << index) != 0; let toggle = weak.clone();
            // Each ID is a fixed protocol name; no row-position state leaks
            // between tools when the catalog is rendered again.
            let button = fold(tool.name, tool.title, expanded, p, window, cx, move |_, cx| { let _ = toggle.update(cx, |this, cx| { this.tools_open ^= 1 << index; cx.notify(); }); })
                .flex_wrap().gap(px(8.0)).child(div().ml_auto().font_family("monospace").text_size(px(10.0)).text_color(p.text_tertiary).child(tool.name));
            div().px(px(12.0)).when(index > 0, |row| row.border_t_1().border_color(p.border_light)).child(button)
                .when(expanded, |row| row.child(note(tool.description, p).pl(px(20.0)).mb(px(12.0))))
        }).collect::<Vec<_>>();
        let push = weak.clone(); let prompt = weak.clone();
        let mut content = div().px(px(4.0)).pb(px(4.0))
            .child(note(copy::LOCAL_NOTICE, p).mb(px(12.0))).child(note(copy::ACCESS_NOTICE, p).mb(px(12.0)))
            .child(div().mb(px(12.0)).border_1().border_color(p.border_medium).rounded(px(4.0)).bg(p.bg_primary).children(tools))
            .child(note(copy::OCR_NOTICE, p).mb(px(12.0)))
            .child(div().border_t_1().border_color(p.border_light).child(fold("mcp-push", copy::PUSH_TITLE, self.push_open, p, window, cx, move |window, cx| {
                let _ = push.update(cx, |this, cx| { this.push_open = !this.push_open; this.push_code = if this.push_open { Some(code_state(guide::push_example(), window, cx)) } else { None }; cx.notify(); });
            })));
        if self.push_open {
            if self.push_code.is_none() { self.push_code = Some(code_state(guide::push_example(), window, cx)); }
            let mut details = div().px(px(20.0)).child(note(copy::PUSH_DESCRIPTION, p).mb(px(12.0))).child(note(copy::PUSH_CONNECTION, p).mb(px(12.0)));
            if let Some(code) = &self.push_code { details = details.child(code_block(code, p).mb(px(12.0))); }
            content = content.child(details.child(note(copy::PUSH_CANCEL, p).mb(px(12.0))).child(note(copy::PUSH_COMPATIBILITY, p).mb(px(12.0))).child(note(copy::CHANGES_NOTICE, p).mb(px(12.0))));
        }
        content = content.child(div().border_t_1().border_color(p.border_light).child(fold("mcp-prompt", copy::PROMPT_TITLE, self.prompt_open, p, window, cx, move |_, cx| { let _ = prompt.update(cx, |this, cx| { this.prompt_open = !this.prompt_open; cx.notify(); }); })));
        if self.prompt_open { content = content.child(note(copy::PROMPT, p).px(px(20.0)).mb(px(12.0))); }
        root = root.child(content); root
    }
}
