//! Provider registry and request plumbing.

use gpui::{App, AppContext as _, Task, TestAppContext};
use gpui_monaco::TextModel;
use gpui_monaco::providers::*;
use gpui_monaco::text::PointUtf16;

struct Words(&'static [&'static str]);

impl CompletionProvider for Words {
    fn trigger_characters(&self) -> Vec<char> {
        vec!['.']
    }

    fn provide_completions(
        &self,
        context: &ProviderContext,
        position: PointUtf16,
        _trigger: CompletionTrigger,
        _cx: &mut App,
    ) -> Task<anyhow::Result<CompletionList>> {
        let line = context.snapshot.line(position.row);
        let items = self
            .0
            .iter()
            .map(|w| {
                CompletionItem::new(format!("{w}:{}", line.len()), CompletionItemKind::Keyword)
            })
            .collect();
        Task::ready(Ok(CompletionList {
            items,
            incomplete: false,
        }))
    }
}

#[gpui::test]
async fn providers_are_selected_by_language_and_unregister(cx: &mut TestAppContext) {
    let (rust, all) = cx.update(|cx| {
        (
            register_completion_provider(cx, LanguageSelector::language("rust"), Words(&["fn"])),
            register_completion_provider(cx, LanguageSelector::All, Words(&["any"])),
        )
    });
    cx.update(|cx| {
        assert_eq!(completion_providers(cx, Some("rust")).len(), 2);
        assert_eq!(completion_providers(cx, Some("json")).len(), 1);
        assert_eq!(completion_providers(cx, None).len(), 1);
    });
    let model = cx.new(|_| {
        TextModel::new("let x")
            .with_language("rust")
            .with_uri("file:///a.rs")
    });
    let task = cx.update(|cx| {
        let context = model.read(cx).provider_context();
        assert_eq!(context.uri.as_deref(), Some("file:///a.rs"));
        let provider = completion_providers(cx, Some("rust")).remove(0);
        provider.provide_completions(
            &context,
            PointUtf16::new(0, 5),
            CompletionTrigger::Invoke,
            cx,
        )
    });
    let list = task.await.unwrap();
    assert_eq!(list.items[0].label.as_ref(), "fn:5");
    cx.update(|cx| {
        unregister_provider(cx, rust);
        unregister_provider(cx, all);
        assert!(completion_providers(cx, Some("rust")).is_empty());
    });
}
