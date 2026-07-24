# Project sidebar and right tool rail

## Summary

Replace twarp's horizontal tab strip with a full-height Projects sidebar modeled on the restraint and navigation clarity of the Codex desktop app. Folder-backed projects form a persistent app-wide library visible in every window, while each open tab remains that window's live project instance. Projects can start and surface chats in their directory context, and Files, Search, and Code Review move into a mutually exclusive right-side tool rail so the center workspace gains vertical room and remains focused on the active pane.

## Problem

The current shell spreads project navigation and project tools across three edges: tabs occupy the top, Files occupies the left, and Code Review occupies the right. This costs vertical space, allows both tools to squeeze the workspace at once, and gives the left sidebar to a secondary tool rather than the user's primary unit of navigation.

The Codex desktop sidebar demonstrates a calmer hierarchy: primary work is selected from a persistent source list, global actions have stable homes, and contextual tools do not compete with navigation. Twarp should adopt that hierarchy without copying Codex-specific destinations or losing tab behavior that users already rely on.

## Goals / Non-goals

**Goals**

- Make projects the primary navigation surface and show each open tab as one chat directly beneath its assigned project.
- Keep folder-backed projects available across relaunches and in every new or existing window, even when no tab for that project is open locally.
- Let users create a project from an existing directory and start a new chat directly in any project's directory context.
- Remove the horizontal tab strip without replacing it with another full-width top bar.
- Move Files, Search, and Code Review to a single right-side tool host with VS Code-style icon toggles.
- Preserve tab identity and behavior: running processes, pane layouts, names, colors, order, restore, close, drag, and cross-window movement.
- Preserve Files, project search, Timeline, Code Review, and their existing project-specific state.
- Make the shell calm, compact, keyboard-accessible, and visually consistent in light and dark themes.

**Non-goals**

- Scratch projects without an assigned directory remain window-local until they gain a folder identity; they do not create permanent anonymous library entries.
- The global project library does not mirror one live pane tree into multiple windows. Opening a library project creates a normal live tab in that window, while another window's instance remains independent.
- No hierarchy deeper than Project → Chat. Panes, files, terminals, and agent sessions inside a tab never become additional sidebar levels.
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

9. The narrow right activity strip remains visible when no right tool is open, so Files, Search, and Code Review have stable, discoverable entry points. It is hidden only when an existing distraction-free or zen mode hides application chrome.

10. Opening both a left Projects sidebar and a right tool rail never overlays or clips the center workspace. At narrow widths, the center yields until its minimum usable width is reached; responsive behavior then temporarily collapses the right tool rail before allowing unusable center content. This temporary collapse does not overwrite the user's open-tool preference, and the rail returns when the window again has enough width.

11. Entering macOS fullscreen collapses only the traffic-light reservation. Project rows move up to use the newly available space without otherwise changing selection, order, scroll position, or rail widths.

### Projects sidebar anatomy

12. The left sidebar contains no TWARP wordmark, icon, or decorative logo.

13. When macOS traffic lights are visible, the sidebar reserves their hit area at the top. No project row, button, search field, status, or drag target overlaps that area.

14. Below the traffic-light area, the sidebar begins with a compact Projects header. It contains the text label `PROJECTS`, a search affordance, and a create affordance. The sidebar toggle stays pinned beside the traffic lights in both open and closed states, so opening the sidebar never shifts the control.

15. The create affordance opens a compact menu with two project-level choices: `Start from scratch` and `Use an existing folder`. The detailed behavior of both choices is defined in the Project creation and chats section.

16. The Projects list fills the remaining vertical space and scrolls independently. Header controls and the footer remain fixed while a long project list scrolls.

17. The bottom footer contains the Settings affordance. Clicking it opens Settings directly; it does not open an account/avatar dropdown.

18. Update availability and offline state appear in the footer only while relevant. They do not permanently reserve a labeled row when no action or warning exists.

19. Browser creation and other global actions that previously occupied the tab strip remain reachable from the create menu, their existing keybindings, the command palette, or another established home. They do not receive permanent left-sidebar rows unless they become primary navigation destinations.

### Project and tab identity

20. Each assigned directory renders as one project parent row. Every open tab assigned to that directory renders as one direct chat child. A registered directory with no open tabs renders as a library project row with no children. Rootless scratch tabs are grouped under a window-local untitled project.

21. Activating a chat child activates its corresponding tab with the live pane tree unchanged. Terminals, agent sessions, browsers, editors, running commands, scroll positions, selections, and split ratios continue from their existing state. Activating an unopened library project creates a normal tab rooted at that directory.

22. The active chat has one clearly selected child row. Its project parent remains visibly identifiable without competing with the active-child selection. Inactive rows use neutral source-list styling with no per-row outlines or divider lines.

23. A project parent and each chat child are one compact line. Chats are always rendered directly below their project; there is no disclosure-created third level and no placeholder such as `No chats` or `No files`.

24. A project parent uses its assigned folder basename, falling back to `Untitled project`. A chat child uses the tab's custom name first and its existing display title second. This preserves every pre-migration tab label as a visible chat title.

25. Duplicate project basenames gain a quiet shortest-useful parent-path sublabel. Chat names may repeat within a project because each maps to a distinct live tab.

26. A tab remains one chat even when its pane tree spans multiple repositories or roots. Its assigned `project_root` determines its project parent; changing an individual pane cwd does not reparent it.

27. Hovering or focusing a truncated project or chat row exposes its complete title and useful folder context without changing row height.

28. Existing colors remain attached to project identity. The project parent renders the small color dot, library rows use the persisted directory color, and every new chat created or promoted into that project inherits the same selected/default colors. A project without an assigned color renders a neutral dot.

29. Each chat child has at most one attention indicator in its trailing status slot. A blocked agent or error outranks passive running activity; the same state is not duplicated on the project parent.

30. Working-tree counts do not appear on project/chat rows or the far-right activity strip. For an agent tab, its three-dot Environment menu shows the changed-file count and available addition/deletion totals beside `Changes`.

31. Hovering a live project parent reveals the single new-chat `+`. Chat children never render a `+`; they reveal only their more-actions control. An unopened library row remains a simple open target.

32. Double-clicking a chat title enters the existing tab rename flow. Rename commits and cancellation behave as they do today and persist across restore.

33. A chat's context menu retains all applicable tab actions, including rename/reset name, close variants, color, metadata copy, save configuration, and sharing. Library rows do not expose tab-only actions.

### Project navigation and manipulation

34. Clicking a chat activates it immediately. Clicking a live project parent keeps or activates its current chat. Clicking an unopened library row opens that directory locally. Clicking a child menu performs only that action and does not trigger the parent.

35. Chat children can be reordered through the existing tab drag behavior and keep that order across restore. Unopened library projects remain ordered by most-recent use and are not draggable live views.

36. Existing cross-window tab drag continues to move a chat's live pane tree and assigned project identity without restarting processes.

37. If a drag is cancelled or released on an invalid target, the project returns to its prior position with no duplication, closure, or state loss.

38. Closing the active chat activates the nearest surviving chat. Closing a folder-backed project's final local chat does not delete the library project; it returns to its unopened state in that window.

39. When the final live project closes in a context that permits an empty window, the center shows the existing welcome/new-session surface while registered library projects remain selectable. `No projects yet` with a single `New project` action appears only when there are neither live projects nor registered folder-backed projects. When the platform or user setting closes the window with its final tab, that behavior remains unchanged.

40. Existing project/tab navigation shortcuts continue to work without a visible horizontal strip, including next/previous project, direct numeric selection where supported, recently used switching, new project, close project, and reopen closed project.

41. When keyboard focus is in the Projects list, Up and Down move the focused row, Enter or Space activates it, the context-menu key opens its actions, and Escape returns focus to the active workspace without changing projects.

42. Project rows expose accessible names that include the full title and, when present, attention state and disambiguating folder context. Color alone is never the only way to distinguish selection or status.

### Project creation and chats

43. The Projects-header create menu presents `Start from scratch` first and `Use an existing folder` second. Both entries have icons, accessible names, keyboard focus states, and tooltips or descriptions when their purpose is not clear from the label alone.

44. Choosing `Start from scratch` creates and activates a standard untitled project, then shows the existing welcome/new-session surface. Terminal, agent, browser, file, worktree, tab-configuration, and other enabled creation paths remain available from that surface, the command palette, existing shortcuts, or their other established homes.

45. Choosing `Use an existing folder` opens the native directory picker. After the user selects a readable directory, twarp registers it in the app-wide project library, creates and activates a new project backed by that directory in the current window, uses its basename as the initial project title, and shows the welcome/new-session surface with the selected directory already set as the project context. Every open window reflects the new library entry without relaunching.

46. Cancelling the directory picker creates nothing and preserves the previously active project, focus, project order, and scroll position.

47. If a selected directory is missing, unreadable, or cannot be opened, no partial project remains in the sidebar. The user sees a concise error and can retry the picker or cancel back to the unchanged Projects list.

48. Selecting a directory already used by another open project creates another independent project, consistent with invariant 20. Renaming either project changes only its display title and never renames or moves the directory.

49. A live project with one unambiguous directory exposes a single `New chat` action on its parent row. Chat rows do not expose this action.

50. Invoking `New chat` creates and activates a fresh tab directly under the project, focuses its empty agent composer, and uses the project's directory as the initial working directory. It does not reuse, clear, replace, or split an existing tab.

51. Before the first message is sent, the composer visibly identifies the inherited project directory. Sending the first message requires no additional directory selection or manual `cd` step.

52. Commands, file references, and agent operations in a newly created chat resolve from the inherited project directory unless the user explicitly changes that chat's working context. Changing a chat's working context does not reassign or rename the project.

53. A project always shows its open tabs as compact, direct chat children. A tab with several panels is still one chat row; its panes never appear as nested sidebar rows.

54. Clicking a chat child activates its tab and resumes its existing state rather than creating a copy. The active chat has a clear selected state, and a long title truncates with its full title available on hover/focus.

55. A project with no open chats renders no placeholder child row. The project parent remains compact.

56a. When the current tab has multiple panels, dragging one panel onto any project promotes that panel into a new tab/chat directly under the target project. The source tab keeps its remaining panels. Single-panel tabs are not split by this gesture, and invalid/cancelled drops restore the source unchanged.

56. For a multi-folder project with no unambiguous primary directory, `New chat` asks the user to choose one of that project's directories before creating the chat. Cancelling creates no chat; an unavailable directory produces a retryable error without changing the active chat.

### Search and global destinations

57. The Projects-header search icon opens the same command palette as Cmd+P. It is not a separate `Search projects` mode and does not replace the header with a project filter field.

58. File search remains Cmd+Shift+F and opens the dedicated Search tool in the right rail. Search and Files never share or overlap the same content surface.

59. Settings is a global destination. Opening Settings creates or focuses its normal settings tab but never adds a project parent or chat child to the Projects hierarchy.

### Right activity strip and tool host

64. The far-right activity strip contains three primary icons in a stable order: Files, Search, and Code Review. Each icon has a tooltip, accessible label, focus state, and active state; icon shape is not the sole active-state signal.

65. Clicking an inactive tool icon opens its content rail and makes it active. Clicking the other icon switches the existing rail directly to that tool without first closing the rail or animating it out and back in.

66. Clicking the already-active tool icon closes the content rail but leaves the activity strip visible. Cmd+Shift+`+` performs the same toggle for the last selected tool; reopening restores that tool, width, and project-specific scroll/selection state.

67. Files, Search, and Code Review are mutually exclusive in the right content rail. They never render side by side, and none can remain in the left Projects sidebar.

68. Files and Code Review remember independent widths. Switching tools does not force the narrower Files preference onto Code Review or the wider Code Review preference onto Files.

69. The right tool rail follows the active project. Switching projects leaves the selected tool and rail width unchanged while replacing its contents with the new active project's Files, Search, or Code Review state.

70. If a newly active project is still resolving its local or remote roots, the selected right tool remains open and shows its existing loading state. The shell does not close, switch tools, or show stale content from the previous project.

71. If the active project does not support the selected tool, the rail shows the tool's existing unavailable/unsupported empty state and any valid recovery action. It does not silently activate the other tool.

72. The existing `Toggle Project Explorer` action opens or focuses Files in the right rail. Invoking it again while Files is active closes the rail.

73. The existing global/project-search action opens or toggles the dedicated Search tool and focuses its query field. Search results remain scoped to the active project.

74. The existing Code Review toggle opens or focuses Code Review in the right rail. Invoking it again while Code Review is active closes the rail.

75. The generic focus-left and focus-right navigation actions target Projects and the active right tool respectively. If the relevant rail is closed, the action opens it before moving focus when that matches the action's current behavior.

### Files tool

76. Files retains the active project's existing roots, expansion state, selection, hidden-file preference, file operations, drag/drop behavior, context menus, and open-in-editor flows.

77. The Files rail header uses the label `FILES` and contains a contextual search affordance that switches to the dedicated Search tool. It does not repeat the project name when that name is already selected in Projects.

78. Opening a file from Files opens or focuses it in the active project's center workspace. It never creates or activates a different project unless the user explicitly chooses an existing new-tab/new-project action.

79. Project-wide search is a dedicated Search destination and third permanent activity icon. Files always gives its full rail height to the file tree and Timeline.

80. Timeline remains part of Files and is collapsed by default. Expanding it reduces only the Files tree's available height, remains independently resizable/scrollable, and does not affect the Projects list or center workspace height.

81. Files uses the entire right rail width and places its scrollbar flush against the rail's content edge. Moving it from left to right does not reverse tree indentation, disclosure direction, filenames, or keyboard navigation.

### Code Review tool

82. Code Review retains repository selection, loading and unsupported states, staged/unstaged sections, multiselect, stage/unstage/discard actions, commit flows, comments, and refresh behavior. A plain file-row click expands or collapses that diff inside the sidebar; opening the full diff in a center tab remains an explicit row action.

83. The Code Review activity icon remains icon-only. Agent tabs surface changed-file and line totals beside `Changes` in their three-dot Environment menu; Code Review may retain detailed statistics inside its own content header.

84. Code Review's content header retains the `CODE REVIEW` label, repository selection when needed, diff statistics, maximize, and contextual actions. The activity icon replaces the header's redundant close button; clicking the active icon closes the rail.

85. Maximizing Code Review expands it across the center workspace while leaving the Projects sidebar and right activity strip available. The active Code Review icon provides a stable way to identify and close or switch away from the maximized tool.

86. Minimizing Code Review returns it to its previously chosen rail width and preserves file selection, scroll position, expanded sections, and review state.

87. Switching from a maximized Code Review to Files exits maximized mode and opens Files at its own saved rail width. Returning to Code Review restores the review content but does not re-maximize it unless the user explicitly does so.

### Global chrome consolidation

88. Command search and file search have distinct homes: the Projects-header icon opens Cmd+P, while the right-rail Search icon and Cmd+Shift+F open project-wide file search.

89. Settings appears once in global shell chrome: in the Projects footer. The removed horizontal strip leaves no duplicate gear or avatar control.

90. Code-change counts are absent from the far-right activity strip and Projects rows. Agent tabs show them in the Environment menu; detailed Code Review content can show its own contextual totals.

91. New-project creation appears once as the Projects-header create affordance. Contextual pane-level creation actions remain where they create a pane or split rather than a project.

92. Removing top-strip controls never removes an action without an alternate visible, menu, palette, menu-bar, or keyboard home. Any action without another discoverable home must be relocated before the strip is removed.

93. In the project-sidebar shell, Projects has a fixed home on the left and the Files/Code Review activity strip has a fixed home on the right. A toolbar-layout editor cannot move these surfaces to another edge or render a second copy.

94. Horizontal-versus-vertical tab placement, vertical-tab presentation modes, and legacy header-toolbar panel-position settings are not presented as active customization options while the project-sidebar shell is enabled. Existing saved values are retained for fallback builds and downgrade compatibility.

### Persistence and transitions

95. Existing open tabs migrate visually to direct chat rows beneath their assigned project without changing order, active selection, names, project colors, pane contents, processes, or restore identity.

96. The Projects sidebar's open/closed state and width are workspace/window-level preferences. Switching projects never changes the left rail's visibility or width.

97. The active right tool, right rail open/closed state, Files width, and Code Review width are workspace/window-level preferences. Tool content such as expanded folders, selected files, selected review repository, and scroll positions remains project-specific where it is project-specific today.

98. Relaunching restores project order, active project, project identity, the app-wide folder library, left-rail visibility and width, active right tool, right-rail visibility, and each tool's width without briefly rendering the old horizontal strip.

99. Runtime rail opening and closing use an approximately 150ms edge transition consistent with the current source rails. Switching Files ↔ Search ↔ Code Review is a direct content transition and does not animate the entire rail off-screen.

100. Rapidly reversing an open/close transition continues from the rail's current visible position without jumping, flashing stale content, or accepting pointer input outside the visible portion.

101. Theme changes, zoom changes, window resizing, and fullscreen transitions preserve active project/tool state. Light and dark themes use adaptive neutral surfaces and semantic colors; no sidebar or rail is pinned to a light-only appearance.

102. If the project-sidebar feature is disabled or the platform is outside its initial scope, the current horizontal-tab and panel layout remains available with no persistence loss. Moving between supported versions does not delete tab, panel, or width data.

### Cross-window project library

103. Folder-backed projects are app-wide identities keyed by their assigned directory. Registering or reopening the same canonical directory refreshes the existing library entry rather than adding duplicate unopened rows.

104. A new window immediately shows the same registered folder-backed projects as every other window, in most-recently-used order, without copying or moving another window's tabs.

105. Opening a library row in one window creates an independent live tab in that window. Closing, splitting, renaming, or changing pane state in that instance does not mutate a live instance in another window.

106. Adding or reopening a folder-backed project updates the Projects list in all currently open windows. Each window preserves its own active tab, focus, search query, scroll position, and sidebar width while incorporating the library change.

107. If a registered directory is missing or unreadable, it remains visible with its stored name and path. Attempting to open it creates no tab and shows the same concise, retryable folder-unavailable error used by folder creation.

108. Scratch projects remain live tab rows only in the window that owns them. They restore with that saved window, but they do not appear as anonymous entries in unrelated new windows.
