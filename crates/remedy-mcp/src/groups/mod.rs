//! The tool groups: thin adapters from MCP calls to `remedy-tools`.

use remedy_tools::Group;

use crate::ToolGroup;

#[cfg(feature = "devtools")]
mod devtools;
#[cfg(feature = "docs")]
mod docs;
#[cfg(feature = "live")]
mod live;
#[cfg(feature = "markup")]
mod markup;
#[cfg(feature = "preview")]
mod preview;
#[cfg(feature = "scaffold")]
mod scaffold;
#[cfg(feature = "testing")]
mod testing;

/// A tool that takes no arguments.
#[allow(dead_code, reason = "builds without the markup and devtools groups")]
#[derive(serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct Nothing {}

/// The group `g`, when this build has it.
pub(crate) fn make(g: Group) -> Option<Box<dyn ToolGroup>> {
    match g {
        #[cfg(feature = "docs")]
        Group::Docs => Some(Box::new(docs::Docs)),
        #[cfg(feature = "markup")]
        Group::Markup => Some(Box::new(markup::Markup)),
        #[cfg(feature = "scaffold")]
        Group::Scaffold => Some(Box::new(scaffold::Scaffold)),
        #[cfg(feature = "preview")]
        Group::Preview => Some(Box::new(preview::Preview)),
        #[cfg(feature = "testing")]
        Group::Testing => Some(Box::new(testing::Testing)),
        #[cfg(feature = "live")]
        Group::Live => Some(Box::new(live::Live)),
        #[cfg(feature = "devtools")]
        Group::Devtools => Some(Box::new(devtools::Devtools)),
        #[allow(unreachable_patterns, reason = "builds without every group")]
        _ => None,
    }
}
