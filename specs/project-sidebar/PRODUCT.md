# Project sidebar and right tool rail

## Summary

Replace twarp's horizontal tab strip with a full-height Projects sidebar modeled on the restraint and navigation clarity of the Codex desktop app. Each existing tab becomes one directory-aware project row that can start and surface chats in that project context, while Files and Code Review move into a mutually exclusive right-side tool rail so the center workspace gains vertical room and remains focused on the active pane.

## Problem

The current shell spreads project navigation and project tools across three edges: tabs occupy the top, Files occupies the left, and Code Review occupies the right. This costs vertical space, allows both tools to squeeze the workspace at once, and gives the left sidebar to a secondary tool rather than the user's primary unit of navigation.

The Codex desktop sidebar demonstrates a calmer hierarchy: primary work is selected from a persistent source list, global actions have stable homes, and contextual tools do not compete with navigation. Twarp should adopt that hierarchy without copying Codex-specific destinations or losing tab behavior that users already rely on.

## Goals / Non-goals

**Goals**

- Make projects the primary navigation surface and give every open tab a stable project row.
- Let users create a project from an existing directory and start a new chat directly in any project's directory context.
- Remove the horizontal tab strip without replacing it with another full-width top bar.
- Move Files and Code Review to a single right-side tool host with VS Code-style icon toggles.
- Preserve tab identity and behavior: running processes, pane layouts, names, colors, order, restore, close, drag, and cross-window movement.
- Preserve Files, project search, Timeline, Code Review, and their existing project-specific state.
- Make the shell calm, compact, keyboard-accessible, and visually consistent in light and dark themes.

**Non-goals**

- No persistent folder registry separate from open tabs. In this version, a project row is a direct presentation of an open tab; choosing a folder creates another tab-backed project.
- No grouping of multiple tabs by repository, even when they use the same folder.
- No nested pane, file, terminal, or generic session rows beneath a project. Only chats associated with that project may appear as child rows.
- No pinned-project or recent-project section in this version.
- No Codex-specific destinations such as Pull requests, Scheduled, or Plugins unless twarp later gains equivalent first-class products.
- No changes to terminal-grid density, pane splitting, agent transcript behavior, file editing, or Code Review operations.
- No mobile/WASM redesign. Windows and Linux retain the existing shell until the project-sidebar behavior is explicitly brought to those platforms.

## Figma

Figma: none provided. The visual references are the owner-supplied twarp and Codex desktop screenshots from 2026-07-23. The behavior in this document takes precedence where the screenshots do not cover a state.

## Relationship to the current shell

This spec supersedes the 2026-07-16 direction that required a horizontal tab strip and prohibited a vertical tab list in the sidebar. It also supersedes the rule that the left full-height rail is the Files/Tools panel. The full-height source-list treatment, traffic-light clearance, restrained color system, right Code Review rail, and window-drag behavior remain desired; their contents and ownership change as specified below.

## Behavior

### Shell structure and space

1. On supported desktop macOS builds, the default shell has three horizontal regions: a left Projects sidebar, the center workspace, and a narrow right activity strip. When a right-side tool is open, its content rail appears between the center workspace and the activity strip.

2. The horizontal tab strip is not rendered. There is no tab-shaped, breadcrumb-shaped, or toolbar-shaped replacement spanning the top of the center workspace.

3. Removing the tab strip returns its full height to the center workspace. Pane content or an existing pane-specific header begins at the top of the center region; the shell does not reserve a blank row where the strip used to be.

4. Pane-specific headers remain where they carry pane-local identity or actions, including editor paths, browser navigation, split-pane controls, and equivalent pane functions. Removing the tab strip does not remove a pane's own required controls.

5. Empty, non-interactive space in the window's top band remains draggable, including empty space over the Projects sidebar and pane headers. Double-clicking an eligible top-band drag region retains the standard macOS window behavior.

6. The Projects sidebar and an open right tool rail are full-height, edge-to-edge source lists. They have neutral surfaces, one hairline at each inner seam, square outer corners, and no docked-card margin or shadow.

7. The left and right content rails remain independently resizable within their existing minimum and maximum width constraints. Their resize handles do not block window dragging, project-row interaction, scrollbars, or the right activity icons.

8. The Projects sidebar can be toggled closed and reopened. Closing it gives its full width back to the center workspace and leaves a compact reopen control adjacent to the traffic-light area; reopening it restores its previous width, scroll position, and focused project row.

9. The narrow right activity strip remains visible when neither Files nor Code Review is open, so both tools have a stable, discoverable entry point. It is hidden only when an existing distraction-free or zen mode hides application chrome.

10. Opening both a left Projects sidebar and a right tool rail never overlays or clips the center workspace. At narrow widths, the center yields until its minimum usable width is reached; responsive behavior then temporarily collapses the right tool rail before allowing unusable center content. This temporary collapse does not overwrite the user's open-tool preference, and the rail returns when the window again has enough width.

11. Entering macOS fullscreen collapses only the traffic-light reservation. Project rows move up to use the newly available space without otherwise changing selection, order, scroll position, or rail widths.

### Projects sidebar anatomy

12. The left sidebar contains no TWARP wordmark, icon, or decorative logo.

13. When macOS traffic lights are visible, the sidebar reserves their hit area at the top. No project row, button, search field, status, or drag target overlaps that area.

14. Below the traffic-light area, the sidebar begins with a compact Projects header. It contains the text label `PROJECTS`, a search affordance, a create affordance, and the sidebar toggle when that control is not already adjacent to the traffic lights.

15. The create affordance opens a compact menu with two project-level choices: `Start from scratch` and `Use an existing folder`. The detailed behavior of both choices is defined in the Project creation and chats section.

16. The Projects list fills the remaining vertical space and scrolls independently. Header controls and the footer remain fixed while a long project list scrolls.

17. The bottom footer contains the Settings affordance. Clicking it opens Settings directly; it does not open an account/avatar dropdown.

18. Update availability and offline state appear in the footer only while relevant. They do not permanently reserve a labeled row when no action or warning exists.

19. Browser creation and other global actions that previously occupied the tab strip remain reachable from the create menu, their existing keybindings, the command palette, or another established home. They do not receive permanent left-sidebar rows unless they become primary navigation destinations.

### Project and tab identity

20. Every open tab renders as exactly one project row, and every project row corresponds to exactly one open tab. Two tabs using the same repository remain two independently selectable rows.

21. Activating a project row activates the corresponding tab with its live pane tree unchanged. Terminals, agent sessions, browsers, editors, running commands, scroll positions, selections, and split ratios continue from their existing state; activation never restarts or recreates the work.

22. The active project has one clearly selected row. Inactive rows use neutral styling; hover and selection follow source-list row anatomy with no per-row outlines or divider lines.

23. A project row is one compact line by default. It contains a fixed identity slot, a single-line title that truncates with an end ellipsis, and a trailing action/status area. Expanded projects may show genuine chat child rows, but never placeholder children such as `No chats` or `No files`.

24. A project row's visible title uses this user-observable priority:
    1. The tab's custom name, when one has been set.
    2. The single project/repository folder name, when one unambiguous folder is known.
    3. The tab's existing display title.
    4. `Untitled project` only when none of the above is available.

25. When multiple open rows would have the same visible title, each duplicate gains a quiet disambiguating sublabel using the shortest useful folder, parent-folder, branch, or ordinal context. Unique rows remain single-line so uncommon collisions do not reduce list capacity for everyone.

26. A tab spanning more than one repository or root remains one project row. Its title follows invariant 24 and its tooltip/accessible description identifies it as a multi-folder project; the sidebar does not split or duplicate the tab.

27. Hovering or focusing a truncated project row exposes the complete title and useful folder/repository context without changing the row's height.

28. Existing per-tab colors remain attached to the corresponding project. The color renders as a small dot in the row's fixed identity slot; it never paints the whole sidebar or competes with semantic success, warning, error, and diff colors. A project without an assigned color renders a neutral dot.

29. Each project row has at most one attention indicator in its trailing status slot. A blocked agent or error outranks passive running activity; the same state is not duplicated elsewhere on that row.

30. Working-tree line counts do not appear on every project row. The active project's current diff count appears on the Code Review activity icon, where it identifies the tool that owns the data.

31. Hovering or keyboard-focusing a project row reveals quiet new-chat and more-actions controls without shifting or truncating the title differently; close remains in the more-actions menu and available through established shortcuts and middle-click behavior.

32. Double-clicking a project title enters the existing rename flow. Rename commits and cancellation behave as they do for tabs today, and the renamed title remains associated with that project across restore.

33. The project context menu retains all applicable tab actions, including rename/reset name, close variants, color, metadata copy, save configuration, and sharing actions that are available for that tab.

### Project navigation and manipulation

34. Clicking a project row activates it immediately. Clicking its close or menu affordance performs only that affordance and never activates the row as a side effect.

35. Project rows can be reordered by dragging vertically. The insertion position is visible, auto-scroll works near the top and bottom edges, and dropping preserves the new order across restore.

36. Dragging a project out of the sidebar can detach it into a new window, and dragging it into another twarp window can insert it into that window's Projects list. The live view tree and project identity move without restarting processes.

37. If a drag is cancelled or released on an invalid target, the project returns to its prior position with no duplication, closure, or state loss.

38. Closing the active project activates the nearest surviving row, preferring the following row and then the previous row. The center workspace and open right tool retarget to the newly active project without closing the shell rails.

39. When the final project closes in a context that permits an empty window, the Projects list shows a compact `No projects open` state with a single `New project` action, and the center shows the existing welcome/new-session surface. When the platform or user setting closes the window with its final tab, that behavior remains unchanged.

40. Existing project/tab navigation shortcuts continue to work without a visible horizontal strip, including next/previous project, direct numeric selection where supported, recently used switching, new project, close project, and reopen closed project.

41. When keyboard focus is in the Projects list, Up and Down move the focused row, Enter or Space activates it, the context-menu key opens its actions, and Escape returns focus to the active workspace without changing projects.

42. Project rows expose accessible names that include the full title and, when present, attention state and disambiguating folder context. Color alone is never the only way to distinguish selection or status.

### Project creation and chats

43. The Projects-header create menu presents `Start from scratch` first and `Use an existing folder` second. Both entries have icons, accessible names, keyboard focus states, and tooltips or descriptions when their purpose is not clear from the label alone.

44. Choosing `Start from scratch` creates and activates a standard untitled project, then shows the existing welcome/new-session surface. Terminal, agent, browser, file, worktree, tab-configuration, and other enabled creation paths remain available from that surface, the command palette, existing shortcuts, or their other established homes.

45. Choosing `Use an existing folder` opens the native directory picker. After the user selects a readable directory, twarp creates and activates a new project backed by that directory, uses its basename as the initial project title, and shows the welcome/new-session surface with the selected directory already set as the project context.

46. Cancelling the directory picker creates nothing and preserves the previously active project, focus, project order, and scroll position.

47. If a selected directory is missing, unreadable, or cannot be opened, no partial project remains in the sidebar. The user sees a concise error and can retry the picker or cancel back to the unchanged Projects list.

48. Selecting a directory already used by another open project creates another independent project, consistent with invariant 20. Renaming either project changes only its display title and never renames or moves the directory.

49. A project with one unambiguous directory exposes a `New chat` action on hover/focus and in its context menu. The same action is keyboard-accessible and is available whether or not that project is active.

50. Invoking `New chat` activates the project, creates a fresh agent chat in its center workspace, focuses the empty composer, and sets the project's directory as the chat's initial working directory. It does not reuse, clear, or replace an existing chat.

51. Before the first message is sent, the composer visibly identifies the inherited project directory. Sending the first message requires no additional directory selection or manual `cd` step.

52. Commands, file references, and agent operations in a newly created chat resolve from the inherited project directory unless the user explicitly changes that chat's working context. Changing a chat's working context does not reassign or rename the project.

53. A project can expand to show its associated chats as compact child rows beneath the project row. Only chats belonging to that project appear there; terminal panes, editors, browsers, and unrelated sessions do not become child rows.

54. Clicking a chat child row activates its project and resumes that chat's existing state rather than creating a copy. The active chat has a clear selected state, and a long chat title truncates with its full title available on hover/focus.

55. A project with no chats renders no placeholder child row. The project row and its `New chat` action remain available, keeping empty projects compact.

56. For a multi-folder project with no unambiguous primary directory, `New chat` asks the user to choose one of that project's directories before creating the chat. Cancelling creates no chat; an unavailable directory produces a retryable error without changing the active chat.

### Project search

57. Invoking search from the Projects header replaces the header label/actions with a focused single-line search field without opening another panel.

58. Search filters the open project rows case-insensitively by custom title, display title, folder/repository name, full path, branch, and active pane title. Matching does not mutate row order.

59. Search results update as the user types. The currently active project remains active even when filtered out; the center workspace never changes merely because a query changes.

60. Up and Down move through matching rows, Enter activates the focused result and closes search, and Escape clears and closes search while preserving the previously active project.

61. A query with no matches shows `No matching projects` and keeps the search field editable. It never offers to search files or create a project implicitly.

62. Clearing the query restores every row and the prior list scroll position. Closing and reopening the Projects sidebar does not preserve a stale query.

63. Command-palette search remains available independently. Project search is scoped only to open project rows and does not replace command, file, setting, or session search.

### Right activity strip and tool host

64. The far-right activity strip contains two primary icons in a stable order: Files and Code Review. Each icon has a tooltip, accessible label, focus state, and active state; icon shape is not the sole active-state signal.

65. Clicking an inactive tool icon opens its content rail and makes it active. Clicking the other icon switches the existing rail directly to that tool without first closing the rail or animating it out and back in.

66. Clicking the already-active tool icon closes the content rail but leaves the activity strip visible. Reopening restores that tool's previous width and project-specific scroll/selection state.

67. Files and Code Review are mutually exclusive in the right content rail. They never render side by side, and neither can remain in the left Projects sidebar.

68. Files and Code Review remember independent widths. Switching tools does not force the narrower Files preference onto Code Review or the wider Code Review preference onto Files.

69. The right tool rail follows the active project. Switching projects leaves the selected tool and rail width unchanged while replacing its contents with the new active project's Files or Code Review state.

70. If a newly active project is still resolving its local or remote roots, the selected right tool remains open and shows its existing loading state. The shell does not close, switch tools, or show stale content from the previous project.

71. If the active project does not support the selected tool, the rail shows the tool's existing unavailable/unsupported empty state and any valid recovery action. It does not silently activate the other tool.

72. The existing `Toggle Project Explorer` action opens or focuses Files in the right rail. Invoking it again while Files is active closes the rail.

73. The existing global/project-search action opens Files in the right rail with its search section expanded and focused. Search results remain scoped to the active project.

74. The existing Code Review toggle opens or focuses Code Review in the right rail. Invoking it again while Code Review is active closes the rail.

75. The generic focus-left and focus-right navigation actions target Projects and the active right tool respectively. If the relevant rail is closed, the action opens it before moving focus when that matches the action's current behavior.

### Files tool

76. Files retains the active project's existing roots, expansion state, selection, hidden-file preference, file operations, drag/drop behavior, context menus, and open-in-editor flows.

77. The Files rail header uses the label `FILES` and contains its contextual search affordance. It does not repeat the project name when that name is already selected in Projects.

78. Opening a file from Files opens or focuses it in the active project's center workspace. It never creates or activates a different project unless the user explicitly chooses an existing new-tab/new-project action.

79. Project-wide search remains a collapsible or otherwise on-demand section within Files rather than a third permanent activity icon in this version. Closing search returns the full rail height to the file tree.

80. Timeline remains part of Files and is collapsed by default. Expanding it reduces only the Files tree's available height, remains independently resizable/scrollable, and does not affect the Projects list or center workspace height.

81. Files uses the entire right rail width and places its scrollbar flush against the rail's content edge. Moving it from left to right does not reverse tree indentation, disclosure direction, filenames, or keyboard navigation.

### Code Review tool

82. Code Review retains repository selection, loading and unsupported states, staged/unstaged sections, multiselect, stage/unstage/discard actions, commit flows, diff opening, comments, and refresh behavior.

83. A compact semantic badge on the Code Review activity icon shows the active project's current additions and deletions. The badge appears only when there are changes, uses semantic diff colors, and is not duplicated in global shell chrome.

84. Code Review's content header retains the `CODE REVIEW` label, repository selection when needed, diff statistics, maximize, and contextual actions. The activity icon replaces the header's redundant close button; clicking the active icon closes the rail.

85. Maximizing Code Review expands it across the center workspace while leaving the Projects sidebar and right activity strip available. The active Code Review icon provides a stable way to identify and close or switch away from the maximized tool.

86. Minimizing Code Review returns it to its previously chosen rail width and preserves file selection, scroll position, expanded sections, and review state.

87. Switching from a maximized Code Review to Files exits maximized mode and opens Files at its own saved rail width. Returning to Code Review restores the review content but does not re-maximize it unless the user explicitly does so.

### Global chrome consolidation

88. Search appears once in global shell chrome: in the Projects header for project filtering. Command search remains available through its shortcut/command palette, and Files search appears only inside Files when invoked.

89. Settings appears once in global shell chrome: in the Projects footer. The removed horizontal strip leaves no duplicate gear or avatar control.

90. Code Review status appears once in global shell chrome: on its right activity icon. A given additions/deletions count is not repeated in the Projects row or a removed top strip.

91. New-project creation appears once as the Projects-header create affordance. Contextual pane-level creation actions remain where they create a pane or split rather than a project.

92. Removing top-strip controls never removes an action without an alternate visible, menu, palette, menu-bar, or keyboard home. Any action without another discoverable home must be relocated before the strip is removed.

93. In the project-sidebar shell, Projects has a fixed home on the left and the Files/Code Review activity strip has a fixed home on the right. A toolbar-layout editor cannot move these surfaces to another edge or render a second copy.

94. Horizontal-versus-vertical tab placement, vertical-tab presentation modes, and legacy header-toolbar panel-position settings are not presented as active customization options while the project-sidebar shell is enabled. Existing saved values are retained for fallback builds and downgrade compatibility.

### Persistence and transitions

95. Existing open tabs migrate visually to project rows without changing their order, active selection, names, colors, pane contents, processes, or restore identity.

96. The Projects sidebar's open/closed state and width are workspace/window-level preferences. Switching projects never changes the left rail's visibility or width.

97. The active right tool, right rail open/closed state, Files width, and Code Review width are workspace/window-level preferences. Tool content such as expanded folders, selected files, selected review repository, and scroll positions remains project-specific where it is project-specific today.

98. Relaunching restores project order, active project, project identity, left-rail visibility and width, active right tool, right-rail visibility, and each tool's width without briefly rendering the old horizontal strip.

99. Runtime rail opening and closing use an approximately 150ms edge transition consistent with the current source rails. Switching Files ↔ Code Review is a direct content transition and does not animate the entire rail off-screen.

100. Rapidly reversing an open/close transition continues from the rail's current visible position without jumping, flashing stale content, or accepting pointer input outside the visible portion.

101. Theme changes, zoom changes, window resizing, and fullscreen transitions preserve active project/tool state. Light and dark themes use adaptive neutral surfaces and semantic colors; no sidebar or rail is pinned to a light-only appearance.

102. If the project-sidebar feature is disabled or the platform is outside its initial scope, the current horizontal-tab and panel layout remains available with no persistence loss. Moving between supported versions does not delete tab, panel, or width data.
