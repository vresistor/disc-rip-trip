//! Hold: keeps whatever arrives until you let it through, then hands it on
//! unchanged. A point in the flow rather than a property of the step after
//! it, so "review, then file" is Review → Hold → Library, and no step needs
//! a setting for it. The same with real and simulated steps.

use anyhow::Result;
use async_trait::async_trait;
use trip_engine::{Cx, Group, Inlet, ItemKind, ItemState, Outlet, Step, StepInfo, StepState};

pub struct Hold;

/// Hold's detail once everything is in and it waits for you to let it through.
pub const WAITS_FOR_YOU: &str = "All here";

#[async_trait]
impl Step for Hold {
    fn info(&self) -> StepInfo {
        StepInfo::new("hold", "Hold", Group::Decide, ItemKind::AnyFile, ItemKind::AnyFile).passes_through().repeatable().from("keeps files until you say they're fine")
    }

    async fn run(&self, mut inlet: Inlet, outlet: Outlet, cx: Cx) -> Result<()> {
        let approve = cx.approval();
        cx.state(StepState::Hold);
        cx.status("Holding", "");
        // The outcome counts what it holds; once all is in, it waits for you.
        let holding = |v: &mut trip_engine::StepView, n: usize, all_in: bool| {
            v.state = StepState::Hold;
            v.status = format!("{n} held");
            v.qualifier.clear();
            v.detail = if all_in { WAITS_FOR_YOU.into() } else { String::new() };
        };
        let (mut held, mut open, mut approved) = (Vec::new(), true, false);
        loop {
            tokio::select! {
                item = inlet.recv(), if open => match item {
                    Some(i) if approved => {
                        cx.item(&i, ItemState::Ok, "let through");
                        outlet.send(i).await;
                    }
                    Some(i) => {
                        cx.item(&i, ItemState::Waiting, "held");
                        held.push(i);
                        let n = held.len();
                        cx.view(|v| holding(v, n, false));
                    }
                    None if approved || cx.stopped() => break,
                    None => {
                        open = false;
                        let n = held.len();
                        cx.view(|v| holding(v, n, true));
                    }
                },
                _ = approve.notified(), if !approved => {
                    approved = true;
                    cx.state(StepState::Busy);
                    let n = held.len();
                    for i in held.drain(..) {
                        cx.item(&i, ItemState::Ok, "let through");
                        outlet.send(i).await;
                    }
                    cx.status(format!("{n} let through"), "");
                    if !open {
                        break;
                    }
                }
                _ = cx.cancelled() => return Ok(()),
            }
        }
        Ok(())
    }
}
