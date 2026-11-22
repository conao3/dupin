# Using Dupin

Dupin lives in the background with no Dock icon or menu bar. Its hotkey (⌘Space by default; see [configuration](./configuration.md)) shows the panel; pressing it again, Esc, or clicking elsewhere hides it.

## Keys

| Key                                     | In the panel                                          |
| --------------------------------------- | ----------------------------------------------------- |
| Hotkey                                  | Show or hide the panel                                |
| Type                                    | Fuzzy-search installed apps                           |
| Space on an empty query                 | Search files with Spotlight, by file and display name |
| `in` then Space                         | Search inside files with Spotlight, as Alfred's `in`  |
| `define` or `d`, then Space             | Look up a word in macOS dictionaries                  |
| Calculation or unit conversion          | Show a calculator result above app matches            |
| ⌫ on an empty query                     | Leave file, content or dictionary search              |
| ↑ ↓                                     | Move the selection                                    |
| Tab at the end of an unselected input   | Complete the selected search keyword                  |
| ↑ on an empty query, then ↑ ↓           | Recall searches in the current mode                   |
| ↩                                       | Open the app or file                                  |
| ⌘↩                                      | Reveal the file in Finder                             |
| → at the end of the input, ⌘K or Ctrl+K | Show actions for the selected result                  |
| Esc, or clicking elsewhere              | Hide the panel                                        |
| ⌘Q                                      | Quit Dupin                                            |
| ⌘,                                      | Open Settings                                         |

Completed searches are remembered for the current Dupin process when you clear
the input, hide the panel or activate a result. With an empty input, ↑ recalls the
latest term in the current mode; further ↑ and ↓ walk through those terms and
back to the empty input. Editing a recalled term restores normal result selection.
This history holds up to 100 terms and is separate from app-ranking history.

## Apps

Dupin finds the `.app` bundles in `/Applications`, `/System/Applications` and `~/Applications`, one folder deep (so `Utilities` is included). An app matches by any name macOS knows it by:

- the name Finder shows, localized (システム設定 for System Settings on a Japanese Mac);
- its file name, when that differs;
- Spotlight's alternate names and keywords: ChatGPT answers to "Codex", its former name, System Settings to "System Preferences", Safari to "browser".

When an app matches through one of those other names, the name shows before its folder in the result.

## Settings

Search for a macOS settings pane, such as `bluetooth`, `displays` or `keyboard`, alongside apps. Enter opens the pane in System Settings. Dupin discovers panes that the installed macOS declares as settings extensions and explicitly allows opening through its settings URL scheme; availability and names follow that macOS version. Searching does not open or change settings.

Type `dupin settings` to open Dupin's own preferences. This uses the same window handoff as ⌘,.

## System commands

Enable System Commands in Settings → Plugins and grant Sleep and/or Lock screen separately. Enabling or granting does not request OS access. Then type `sleep` or `lock` in ordinary search, select Sleep or Lock Screen Now, and press Enter to review its action. Choose Sleep Now or Lock Screen Now on the action screen to execute it; Escape returns to search. Searching and the first Enter never run a command. Sleep requests full machine sleep. Lock Screen sends the standard Control–Command–Q shortcut and requires existing event-synthesis access; a disabled or intercepted shortcut can instead reach the frontmost app and trigger its action. Refusals appear in the launcher.

## Web searches

Type `g rust lifetimes`, `gh dupin` or `yt piano` to search Google, GitHub or YouTube. The search appears as the first result alongside any matching apps; Enter opens it in the default browser. Typing just the keyword shows the search too. Select it and press Tab to add a space before typing the query.

Web searches are local URL templates: typing never sends a request to the engine. The query is encoded into the URL only when the result is built, and the browser opens only after activation. Web results are not saved in usage-ranking history; query recall behaves as it does for other app-mode searches.

Set `web_searches` and optional `web_fallbacks` in [configuration](./configuration.md). Fallback searches appear only after ordinary providers finish without results. They are disabled by default. File and Dictionary modes keep their own providers.

## Maps

Enable **Maps/Places** in Settings → Plugins, then type `maps` or `map` and Space followed by an address or place. Grant **Search in Maps** separately before Enter can open that search in Maps. Without the grant, the query stays in the picker with an explanation.

Searches are handed to Maps when activated. Dupin does not fetch live suggestions or your location while you type. This plugin requires the Wasm-enabled build.

## Files

Space on an empty query switches to file search; a "Files" chip shows the mode. Every word of the query must appear in the file name or display name. Results appear as Spotlight finds them, then improve as more candidates arrive. Ranking combines filename match quality, your usage and previous query choices, last-used and modified dates, and path depth. Cache and build directories are demoted by the configurable [file ranking rules](./configuration.md#file-ranking).

The list shows up to `file_limit` results (200 by default) from a bounded candidate pool. A selected file or an open action screen keeps its target while results reorder.

`in` followed by Space switches to content search ("In files"). Every word must start a word in the text Spotlight indexed from inside the file.

Narrow either file mode with qualifiers:

| Qualifier        | Meaning                                                                     |
| ---------------- | --------------------------------------------------------------------------- |
| `kind:pdf`       | A category: pdf, image, text, audio, video, folder, archive or app          |
| `ext:rs`         | An extension, including compound extensions such as `ext:tar.gz`            |
| `in:~/Documents` | Search within an absolute or home-relative folder                           |
| `modified:7d`    | Modified within a positive number of hours (`h`), days (`d`) or weeks (`w`) |

Combine different qualifiers, using each at most once: `report kind:pdf modified:7d`. Quote folder paths containing spaces, such as `in:"~/My Documents"`. Backslashes can escape spaces, quotes and backslashes in qualifier values. Unknown colon prefixes remain ordinary search text.

Folder scopes resolve symlinks and require an existing directory. A missing path or a path to a file returns no results.

Valid qualifiers appear as inline chips without changing the query text. Backspace deletes a whole chip, and Undo restores it. Copy and query recall retain the raw query. Incomplete or invalid qualifiers show a syntax hint and return no results. File-name search accepts qualifiers without ordinary terms; content search still requires a word to search for.

Both modes search the Spotlight index, so they find only what Spotlight has indexed.
Right Arrow at the end of an unselected input, or Cmd+K, opens the selected
result's action screen: Open, Reveal in Finder and Copy path. Escape cancels the
screen and returns to the exact search and selection. Use ↑ and ↓ to select an action, Enter to run it, and Left or Escape to return. Ctrl+K also opens actions. The same screen is available for app results.

`define` or `d` followed by Space switches to Dictionary Services ("Define"). Type a word to see its definition from the dictionaries enabled in Dictionary.app. Right Arrow, Cmd+K, Enter or the Actions link opens its action screen; choose Open in Dictionary to launch the external app. Backspace on an empty query returns to app search. Set `dictionary_keywords` in [configuration](./configuration.md) to choose other aliases.

Dictionary is disabled by default. Enable Dictionary in Settings → Plugins to use its keyword lookups. Grant **Open in Dictionary** separately to use that action; enabling lookup does not grant URL access.

Arithmetic and local unit conversions appear as a source/result card alongside
app search; no keyword is needed. Enter copies the result only after **Copy
results** has been granted in Settings → Plugins → Calculator. Without that
grant, the query remains in place and the panel explains where to enable copying.
Currency conversion is not available in the launcher yet.

You can also paste a complete query such as `in release notes` or `d serendipity` into app search. Dupin accepts the same keywords and configured aliases, switches mode, and leaves the search term in the input.
