//! App-wide settings: required ones keep a recipe from starting until set,
//! once for every recipe, and a running step reads them like any other.

use anyhow::Result;
use async_trait::async_trait;
use drt_engine::recipe::chain;
use drt_engine::setting::ParamKind;
use drt_engine::{Cx, Engine, Group, Inlet, ItemKind, Node, Outlet, ParamSpec, Recipe, Registry, Setting, Step, StepInfo};
use serde_json::Value;

struct Lookup;

#[async_trait]
impl Step for Lookup {
    fn info(&self) -> StepInfo {
        StepInfo::new("lookup", "Lookup", Group::Source, ItemKind::Nothing, ItemKind::Nothing)
            .param(ParamSpec::new("api_key", "API key", ParamKind::Text).app_wide())
            .summary(|g| g("api_key").and_then(|v| v.as_str().map(|s| format!("key {s}"))).unwrap_or_else(|| "no key".into()))
    }

    async fn run(&self, mut inlet: Inlet, _outlet: Outlet, cx: Cx) -> Result<()> {
        inlet.recv().await;
        cx.status(cx.text("api_key"), "");
        Ok(())
    }
}

fn recipe(name: &str) -> Recipe {
    Recipe { name: name.into(), root: chain([Node::step("l", "lookup")]) }
}

#[tokio::test]
async fn an_app_wide_setting_is_asked_for_once_and_shared_by_every_recipe() {
    let mut reg = Registry::default();
    reg.add(Lookup);
    let mut e = Engine::new(reg);
    e.time_scale = 0.0;
    e.work_root = std::env::temp_dir().join(format!("drt-app-scope-{}", std::process::id()));

    let missing = e.missing(&recipe("a"));
    assert_eq!(missing.len(), 1);
    assert_eq!((missing[0].step.as_str(), missing[0].key), ("l", "api_key"));
    assert!(e.start(&recipe("a")).is_err());
    assert_eq!(e.idle(&recipe("a")).steps["l"].from, "no key");

    e.settings.lock().unwrap().set_app("lookup", "api_key", Setting::chosen(Value::from("abc")));
    // A recipe-scoped value of the same name is not what's used.
    let other = Recipe { name: "b".into(), root: chain([Node::step("l", "lookup").with("api_key", "wrong")]) };
    for r in [recipe("a"), other] {
        assert!(e.missing(&r).is_empty());
        assert_eq!(e.idle(&r).steps["l"].from, "key abc");
        let s = e.start(&r).unwrap().finished().await;
        assert_eq!(s.steps["l"].status, "abc");
    }
}
