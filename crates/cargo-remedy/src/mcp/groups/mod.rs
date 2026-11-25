//! The tool groups: thin adapters from MCP calls to `remedy-tools`.

use crate::devkit::Group;

use crate::mcp::ToolGroup;

#[cfg(feature = "mcp-devtools")]
mod devtools;
#[cfg(feature = "mcp-docs")]
mod docs;
#[cfg(feature = "mcp-effects")]
mod effects;
#[cfg(feature = "mcp-live")]
mod live;
#[cfg(feature = "mcp-markup")]
mod markup;
#[cfg(feature = "mcp-preview")]
mod preview;
#[cfg(feature = "mcp-scaffold")]
mod scaffold;
#[cfg(feature = "mcp-testing")]
mod testing;

/// A tool that takes no arguments.
#[allow(dead_code, reason = "builds without the markup and devtools groups")]
#[derive(serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct Nothing {}

/// The group `g`, when this build has it.
pub(crate) fn make(g: Group) -> Option<Box<dyn ToolGroup>> {
    match g {
        #[cfg(feature = "mcp-docs")]
        Group::Docs => Some(Box::new(docs::Docs)),
        #[cfg(feature = "mcp-markup")]
        Group::Markup => Some(Box::new(markup::Markup)),
        #[cfg(feature = "mcp-effects")]
        Group::Effects => Some(Box::new(effects::Effects)),
        #[cfg(feature = "mcp-scaffold")]
        Group::Scaffold => Some(Box::new(scaffold::Scaffold)),
        #[cfg(feature = "mcp-preview")]
        Group::Preview => Some(Box::new(preview::Preview)),
        #[cfg(feature = "mcp-testing")]
        Group::Testing => Some(Box::new(testing::Testing)),
        #[cfg(feature = "mcp-live")]
        Group::Live => Some(Box::new(live::Live)),
        #[cfg(feature = "mcp-devtools")]
        Group::Devtools => Some(Box::new(devtools::Devtools)),
        #[allow(unreachable_patterns, reason = "builds without every group")]
        _ => None,
    }
}
