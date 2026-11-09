//! Browser: web links you Ctrl+click (in the terminal, the editor, and
//! previews) open in a tab that shows the page, instead of in your browser.
//! It's the two halves of Flareite's API 1.15 that make that work:
//!
//! - a **link handler** (`[[link_handlers]]`): Flareite runs its command
//!   with the link (`links::opening`), and
//! - a **layout** with a **web page pane**: a browser view of its own in the
//!   tab, which tells the extension where it goes (`layouts::navigated`) and
//!   which the extension moves (`layouts::navigate-pane`), with buttons in
//!   the pane's header.

use std::cell::RefCell;
use std::collections::HashMap;

use flareite_extension_api::layouts::{
  self, Button, FillReason, Layout, LayoutNode, LayoutTab, NodeKind, Pane, PaneContent, PaneHeader,
  WebAction,
};
use flareite_extension_api::prompts::{self, AnswerValue, InputOptions};
use flareite_extension_api::types::{
  Activation, CommandContext, DocumentChange, DocumentId, DocumentInfo, LogLevel, MessageLevel,
};
use flareite_extension_api::{Guest, export_extension, host, links};

const LAYOUT: &str = "page";
const PANE: &str = "page";

/// What an open tab shows now.
struct Page {
  url: String,
  title: Option<String>,
  loading: bool,
}

#[derive(Default)]
struct State {
  /// Open tabs, by key: the address each started at.
  pages: HashMap<String, Page>,
  /// The address asked for, and the tab it's for (`None`: a new tab).
  asking: Option<(u64, Option<String>)>,
}

thread_local! {
  static STATE: RefCell<State> = RefCell::new(State::default());
}

fn tab(key: &str) -> LayoutTab {
  LayoutTab {
    layout: LAYOUT.into(),
    key: key.into(),
  }
}

fn log(message: &str) {
  host::log(LogLevel::Warn, message);
}

/// An address as typed: `example.com` means `https://example.com`.
fn address(text: &str) -> Option<String> {
  let text = text.trim();
  if text.is_empty() || text.contains(char::is_whitespace) {
    return None;
  }
  let lower = text.to_ascii_lowercase();
  Some(
    if lower.starts_with("https://") || lower.starts_with("http://") {
      text.to_owned()
    } else {
      format!("https://{text}")
    },
  )
}

/// The site's name in an address, for a tab's title before the page has one.
fn site(url: &str) -> &str {
  let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
  rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// Opens an address in a tab of its own (or shows its tab).
fn open_tab(url: String) {
  STATE.with_borrow_mut(|state| {
    state.pages.entry(url.clone()).or_insert(Page {
      url: url.clone(),
      title: None,
      loading: true,
    });
  });
  if let Err(problem) = layouts::open(&tab(&url), false) {
    log(&format!("can't open a tab for {url}: {problem:?}"));
  }
}

fn button(id: &str, title: &str, icon: &str, command: &str) -> Button {
  Button {
    id: id.into(),
    title: title.into(),
    icon: Some(icon.into()),
    command: format!("flareite.browser.{command}"),
    argument: None,
    primary: false,
    pressed: None,
    disabled: false,
  }
}

/// A tab's layout: the page, under a header with where it is and buttons.
/// The pane keeps the address it started at, so laying the tab out again
/// (as the title changes) doesn't move the page.
fn lay_out(key: &str) {
  let Some((url, title, loading)) = STATE.with_borrow(|state| {
    let page = state.pages.get(key)?;
    Some((page.url.clone(), page.title.clone(), page.loading))
  }) else {
    return;
  };
  let title = title
    .filter(|title| !title.trim().is_empty())
    .unwrap_or_else(|| site(&url).to_owned());
  let layout = Layout {
    title: title.chars().take(200).collect(),
    nodes: vec![LayoutNode {
      id: PANE.into(),
      parent: None,
      size: 1.0,
      kind: NodeKind::Pane(Pane {
        content: PaneContent::WebPage(key.into()),
        header: Some(PaneHeader {
          title: if loading { "Loading…" } else { "Page" }.into(),
          description: Some(url.chars().take(500).collect()),
          buttons: vec![
            button("back", "Back", "chevron-left", "back"),
            button("forward", "Forward", "chevron-right", "forward"),
            button("reload", "Reload", "refresh", "reload"),
            button("go", "Go to an Address…", "search", "goTo"),
            button("external", "Open in Your Browser", "open", "external"),
          ],
        }),
        scroll_group: None,
      }),
    }],
    documents: Vec::new(),
    buttons: Vec::new(),
    breakpoints: Vec::new(),
    main: Some(PANE.into()),
  };
  if let Err(problem) = layouts::set_layout(&tab(key), &layout) {
    log(&format!("can't show {key}: {problem:?}"));
  }
}

/// The tab a button was clicked in.
fn clicked() -> Option<String> {
  layouts::invoked().map(|from| from.tab.key)
}

fn act(action: WebAction) {
  if let Some(key) = clicked() {
    let _ = layouts::navigate_pane(&tab(&key), PANE, &action);
  }
}

/// Asks for an address: for the tab clicked in, or (`None`) a new tab.
fn ask(for_tab: Option<String>) {
  let value = for_tab
    .as_ref()
    .and_then(|key| STATE.with_borrow(|state| Some(state.pages.get(key)?.url.clone())))
    .unwrap_or_default();
  let options = InputOptions {
    title: "Go to an address".into(),
    value,
    placeholder: Some("example.com".into()),
  };
  match prompts::input(&options, "flareite.browser.answered") {
    Ok(prompt) => STATE.with_borrow_mut(|state| state.asking = Some((prompt, for_tab))),
    Err(problem) => log(&format!("can't ask for an address: {problem:?}")),
  }
}

fn answered() {
  let Some(answer) = prompts::answer() else {
    return;
  };
  let Some((_, for_tab)) =
    STATE.with_borrow_mut(|state| state.asking.take_if(|(prompt, _)| *prompt == answer.prompt))
  else {
    return;
  };
  let AnswerValue::Text(text) = answer.value else {
    return;
  };
  let Some(url) = address(&text) else {
    host::show_message(MessageLevel::Warning, &format!("{text} isn't an address."));
    return;
  };
  match for_tab {
    Some(key) => {
      let _ = layouts::navigate_pane(&tab(&key), PANE, &WebAction::Go(url));
    }
    None => open_tab(url),
  }
}

/// The layout's command: fills a tab in, or forgets it.
fn fill() {
  let Some(request) = layouts::filling() else {
    return;
  };
  let key = request.tab.key;
  match request.reason {
    FillReason::Opened | FillReason::Restored => {
      STATE.with_borrow_mut(|state| {
        state.pages.entry(key.clone()).or_insert(Page {
          url: key.clone(),
          title: None,
          loading: true,
        });
      });
      lay_out(&key);
    }
    FillReason::Closed => {
      STATE.with_borrow_mut(|state| state.pages.remove(&key));
    }
    FillReason::Resized | FillReason::Ready => {}
  }
}

/// A page went somewhere, finished loading, or got its title.
fn navigated() {
  let Some(navigation) = layouts::navigated() else {
    return;
  };
  let key = navigation.tab.key;
  let changed = STATE.with_borrow_mut(|state| {
    let page = state.pages.get_mut(&key)?;
    let before = (page.url.clone(), page.title.clone(), page.loading);
    if page.url != navigation.url {
      // A new page: its title comes later.
      page.title = None;
    }
    page.url = navigation.url;
    if navigation.title.is_some() {
      page.title = navigation.title;
    }
    page.loading = navigation.loading;
    Some(before != (page.url.clone(), page.title.clone(), page.loading))
  });
  if changed == Some(true) {
    lay_out(&key);
  }
}

struct Browser;

impl Guest for Browser {
  fn activate(_: Activation) -> Result<(), String> {
    Ok(())
  }

  fn deactivate() {}

  fn run_command(command: String, _: CommandContext) -> Result<(), String> {
    match command.trim_start_matches("flareite.browser.") {
      "open" => {
        if let Some(link) = links::opening() {
          open_tab(link.target);
        }
      }
      "openAddress" => ask(None),
      "answered" => answered(),
      "fill" => fill(),
      "navigated" => navigated(),
      "back" => act(WebAction::Back),
      "forward" => act(WebAction::Forward),
      "reload" => act(WebAction::Reload),
      "goTo" => ask(clicked()),
      "external" => {
        let url = clicked()
          .and_then(|key| STATE.with_borrow(|state| Some(state.pages.get(&key)?.url.clone())));
        if let Some(url) = url
          && let Err(problem) = links::open_in_browser(&url)
        {
          log(&format!("can't open {url}: {problem:?}"));
        }
      }
      other => return Err(format!("unknown command {other}")),
    }
    Ok(())
  }

  fn document_opened(_: DocumentInfo) {}

  fn document_changed(_: DocumentChange) {}

  fn document_closed(_: DocumentId) {}
}

export_extension!(Browser);

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn addresses_are_completed() {
    assert_eq!(
      address("example.com").as_deref(),
      Some("https://example.com")
    );
    assert_eq!(
      address(" http://a.org/x ").as_deref(),
      Some("http://a.org/x")
    );
    assert_eq!(address("two words"), None);
    assert_eq!(site("https://example.com/a?b"), "example.com");
  }
}
