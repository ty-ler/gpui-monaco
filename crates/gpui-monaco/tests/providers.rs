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

/// A hover provider that only says who it is.
struct Named(&'static str);

impl HoverProvider for Named {
    fn provide_hover(
        &self,
        _: &ProviderContext,
        _: PointUtf16,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        Task::ready(Ok(Some(Hover {
            contents: vec![self.0.into()],
            ..Default::default()
        })))
    }
}

/// The names of the hover providers for `target`, in lookup order.
async fn hover_order(cx: &mut TestAppContext, target: ProviderTarget) -> Vec<String> {
    let model = cx.new(|_| TextModel::new("x"));
    let tasks = cx.update(|cx| {
        let context = model.read(cx).provider_context();
        hover_providers(cx, target)
            .into_iter()
            .map(|provider| provider.provide_hover(&context, PointUtf16::default(), cx))
            .collect::<Vec<_>>()
    });
    let mut names = Vec::new();
    for task in tasks {
        let hover = task.await.unwrap().unwrap();
        names.push(hover.contents[0].value.to_string());
    }
    names
}

fn target(language: &str, uri: &str) -> ProviderTarget {
    ProviderTarget {
        language_id: Some(language.to_string().into()),
        uri: Some(uri.to_string().into()),
    }
}

/// Monaco `LanguageFeatureRegistry._compareByScoreAndTime`: the best score
/// first (an exact language 10, `*` 5), then the newest registration.
#[gpui::test]
async fn providers_are_ranked_by_score_then_newest(cx: &mut TestAppContext) {
    cx.update(|cx| {
        register_hover_provider(cx, LanguageSelector::All, Named("all, oldest"));
        register_hover_provider(cx, LanguageSelector::language("rust"), Named("rust, old"));
        register_hover_provider(
            cx,
            LanguageSelector::Languages(vec!["*".into()]),
            Named("star"),
        );
        register_hover_provider(cx, LanguageSelector::language("rust"), Named("rust, new"));
        register_hover_provider(cx, LanguageSelector::language("json"), Named("json"));
    });
    assert_eq!(
        hover_order(cx, target("rust", "file:///a.rs")).await,
        ["rust, new", "rust, old", "star", "all, oldest"]
    );
    assert_eq!(
        hover_order(cx, target("css", "file:///a.css")).await,
        ["star", "all, oldest"]
    );
}

/// Monaco `isExclusive`: a matching exclusive selector hides the others.
#[gpui::test]
async fn exclusive_selectors_win(cx: &mut TestAppContext) {
    cx.update(|cx| {
        register_hover_provider(cx, LanguageSelector::language("rust"), Named("rust"));
        register_hover_provider(
            cx,
            LanguageSelector::Filters(vec![LanguageFilter {
                scheme: Some("git".into()),
                exclusive: true,
                ..Default::default()
            }]),
            Named("git only"),
        );
    });
    assert_eq!(
        hover_order(cx, target("rust", "git:/a.rs")).await,
        ["git only"]
    );
    assert_eq!(
        hover_order(cx, target("rust", "file:///a.rs")).await,
        ["rust"]
    );
}

/// Monaco `orderedGroups`: equal scores share a group, best group first.
#[gpui::test]
fn completion_providers_come_in_groups(cx: &mut TestAppContext) {
    cx.update(|cx| {
        register_completion_provider(cx, LanguageSelector::All, Words(&["a"]));
        register_completion_provider(cx, LanguageSelector::language("rust"), Words(&["b"]));
        register_completion_provider(cx, LanguageSelector::language("rust"), Words(&["c"]));
        let groups = completion_provider_groups(cx, Some("rust"));
        assert_eq!(groups.iter().map(Vec::len).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(completion_provider_groups(cx, Some("json")).len(), 1);
    });
}
