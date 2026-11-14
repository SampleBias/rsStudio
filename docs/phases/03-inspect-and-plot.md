# Phase 03 — Environment, data, and plots

**Objective:** inspect session objects and view plots without leaving the IDE. **Depends on:** phases 01 and 02.

## Work packages

1. Add an environment panel for explicitly listed global objects with name, class, dimensions/length, and bounded previews. Refresh after user execution or on demand, without concurrent evaluation; handle deleted, unsupported, or problematic objects and active bindings safely.
2. Add a read-only data-frame viewer with row/column names, types, `NA`, paging or virtualization, horizontal/vertical scrolling, and explicit loading/error states. Fetch bounded slices by object identity and page coordinates; do not transfer an entire large data frame to Rust. Detect changed/deleted objects and invalidate stale pages. Define behavior for list columns, factors, dates, Unicode, and unsupported classes.
3. Prototype plot capture through a managed R graphics device and private temporary files; see [architecture](../architecture.md#feature-integration). Support display, resize/refresh where feasible, previous/next captured plot, and export to a user-selected path. Set explicit limits and report unsupported device behavior.
4. Coordinate inspection and plot capture with the session queue and generation IDs. Clean up temporary resources when plots are discarded or the session exits.

## Demo and exit criteria

- In the editor run `df <- data.frame(name = c("a", "b"), value = c(1, NA))`; see it in the environment and inspect both rows and the missing value in the viewer. Change or remove `df` and verify stale pages are not presented as current.
- View a data frame with at least 100,000 rows without transferring or rendering it all at once; document the tested paging and memory behavior. Unsupported objects produce a clear message.
- Run `plot(1:10)` and view and export the image. Validate multiple plots, errors during drawing, explicit device changes, and `ggplot2` when independently installed; document any unsupported cases rather than claiming universal plot capture.
- Automated real-R tests verify inspection and capture; manual tests verify viewport navigation, export failure, large-data responsiveness, and cleanup after restart.
