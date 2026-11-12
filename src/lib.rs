//! Browser: web links you Ctrl+click (in the terminal, the editor, and
//! previews) open in a tab that shows the page, instead of in your browser.
//! It's the two halves of Flareite's API that make that work:
//!
//! - a **link handler** (`[[link_handlers]]`): Flareite asks the extension
//!   to open the link (`open-link`), and
//! - a **layout** with a **web page pane**: a browser view of its own in the
//!   tab, which tells the extension where it goes (`pane-navigated` events)
//!   and which the extension moves (`layouts::navigate-pane`), with buttons
//!   in the pane's header.

use std::collections::HashMap;

use flareite_extension_api::events::Event;
use flareite_extension_api::layouts::{
  self, Button, FillReason, FillRequest, Layout, LayoutNode, Navigation, NodeKind, Pane,
  PaneContent, PaneHeader, WebAction,
};
use flareite_extension_api::links::{self, LinkRequest};
use flareite_extension_api::prompts::{self, AnswerValue, InputOptions, PromptAnswer};
use flareite_extension_api::types::{
  Activation, CommandContext, LayoutTab, LogLevel, MessageLevel,
};
use flareite_extension_api::{Extension, host, register};

const LAYOUT: &str = "page";
const PANE: &str = "page";

/// What an open tab shows now.
struct Page {
  url: String,
  title: Option<String>,
  loading: bool,
}

impl Page {
  fn new(url: &str) -> Self {
    Self {
      url: url.to_owned(),
      title: None,
      loading: true,
    }
  }
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
fn lay_out(key: &str, page: &Page) {
  let title = page
    .title
    .clone()
    .filter(|title| !title.trim().is_empty())
    .unwrap_or_else(|| site(&page.url).to_owned());
  let layout = Layout {
    title: title.chars().take(200).collect(),
    nodes: vec![LayoutNode {
      id: PANE.into(),
      parent: None,
      size: 1.0,
      kind: NodeKind::Pane(Pane {
        content: PaneContent::WebPage(key.into()),
        header: Some(PaneHeader {
          title: if page.loading { "Loading…" } else { "Page" }.into(),
          description: Some(page.url.chars().take(500).collect()),
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

fn act(key: &str, action: WebAction) {
  let _ = layouts::navigate_pane(&tab(key), PANE, &action);
}

#[derive(Default)]
struct Browser {
  /// Open tabs, by key: the address each started at.
  pages: HashMap<String, Page>,
  /// The address asked for, and the tab it's for (`None`: a new tab).
  asking: Option<(u64, Option<String>)>,
}

impl Browser {
  /// Opens an address in a tab of its own (or shows its tab).
  fn open_tab(&mut self, url: String) {
    self
      .pages
      .entry(url.clone())
      .or_insert_with(|| Page::new(&url));
    if let Err(problem) = layouts::open(&tab(&url), false) {
      log(&format!("can't open a tab for {url}: {problem:?}"));
    }
  }

  /// Asks for an address: for a tab, or (`None`) a new tab.
  fn ask(&mut self, for_tab: Option<String>) {
    let value = for_tab
      .as_ref()
      .and_then(|key| Some(self.pages.get(key)?.url.clone()))
      .unwrap_or_default();
    let options = InputOptions {
      title: "Go to an address".into(),
      value,
      placeholder: Some("example.com".into()),
    };
    match prompts::input(&options) {
      Ok(prompt) => self.asking = Some((prompt, for_tab)),
      Err(problem) => log(&format!("can't ask for an address: {problem:?}")),
    }
  }

  fn answered(&mut self, answer: PromptAnswer) {
    let Some((_, for_tab)) = self.asking.take_if(|(prompt, _)| *prompt == answer.prompt) else {
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
      Some(key) => act(&key, WebAction::Go(url)),
      None => self.open_tab(url),
    }
  }

  /// Fills a tab in, or forgets it.
  fn fill(&mut self, request: FillRequest) {
    let key = request.tab.key;
    match request.reason {
      FillReason::Opened | FillReason::Restored => {
        let page = self
          .pages
          .entry(key.clone())
          .or_insert_with(|| Page::new(&key));
        lay_out(&key, page);
      }
      FillReason::Closed => {
        self.pages.remove(&key);
      }
      FillReason::Resized | FillReason::Ready => {}
    }
  }

  /// A page went somewhere, finished loading, or got its title.
  fn navigated(&mut self, navigation: Navigation) {
    let key = navigation.tab.key;
    let Some(page) = self.pages.get_mut(&key) else {
      return;
    };
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
    if before != (page.url.clone(), page.title.clone(), page.loading) {
      lay_out(&key, page);
    }
  }
}

impl Extension for Browser {
  fn activate(_: Activation) -> Result<Self, String> {
    Ok(Self::default())
  }

  fn run_command(&mut self, command: &str, context: CommandContext) -> Result<(), String> {
    // The buttons' commands are for the tab they were clicked in.
    let clicked = context.invoked_from.map(|from| from.tab.key);
    match command.trim_start_matches("flareite.browser.") {
      "openAddress" => self.ask(None),
      "goTo" => self.ask(clicked),
      "back" | "forward" | "reload" | "external" => {
        let key = clicked.ok_or("click the button in a browser tab")?;
        match command.trim_start_matches("flareite.browser.") {
          "back" => act(&key, WebAction::Back),
          "forward" => act(&key, WebAction::Forward),
          "reload" => act(&key, WebAction::Reload),
          _ => {
            if let Some(page) = self.pages.get(&key)
              && let Err(problem) = links::open_in_browser(&page.url)
            {
              log(&format!("can't open {}: {problem:?}", page.url));
            }
          }
        }
      }
      other => return Err(format!("unknown command {other}")),
    }
    Ok(())
  }

  fn handle_event(&mut self, event: Event) -> Result<(), String> {
    match event {
      Event::PromptAnswered(answer) => self.answered(answer),
      Event::LayoutFill(request) => self.fill(request),
      Event::PaneNavigated(navigation) => self.navigated(navigation),
      _ => {}
    }
    Ok(())
  }

  fn open_link(&mut self, request: LinkRequest) -> Result<bool, String> {
    self.open_tab(request.link.target);
    Ok(true)
  }
}

register!(Browser);

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
