use std::sync::Arc;
use std::{collections::HashSet, ops::RangeInclusive};

use itertools::Itertools as _;

use twarpui::elements::SecretRange;

use crate::terminal::model::grid::{grapheme_cursor, Dimensions as _};
use crate::terminal::model::secrets::SecretsRegex;
use crate::terminal::model::terminal_model::RangeInModel;
use crate::terminal::model::{
    grid::RespectDisplayedOutput,
    index::{Direction, Point},
    secrets::{
        IsObfuscated, ObfuscateSecrets, Secret, SecretAndHandle, SecretHandle, SecretLevel,
        SECRETS_REGEX,
    },
};

use super::GridHandler;

// twarp: ported from upstream's `ai::blocklist::block::secret_redaction` (deleted in this
// fork with the agent-mode UI) so grid secret-level detection can re-scan plaintext
// against a consistent `SecretsRegex` snapshot.
pub(crate) fn find_secrets_in_text_with_levels_using_regex(
    text: &str,
    secrets_regex: &SecretsRegex,
) -> Vec<(SecretRange, SecretLevel)> {
    let SecretsRegex {
        regex,
        level_metadata,
        ..
    } = secrets_regex;

    let mut secret_ranges = vec![];
    let mut byte_to_char_index = vec![0; text.len() + 1]; // Map byte index to char index

    // Track the current character index while iterating through the string.
    let mut char_index = 0;
    for (byte_index, _) in text.char_indices() {
        byte_to_char_index[byte_index] = char_index;
        char_index += 1;
    }
    byte_to_char_index[text.len()] = char_index; // Map the last byte to the last character index

    // Iterate over the text once, finding all matches against secret regex. Map the byte ranges
    // to character ranges and store them.
    for mat in regex.find_iter(text) {
        let start_byte = mat.start();
        let end_byte = mat.end();
        let start_char = byte_to_char_index[start_byte];
        let end_char = byte_to_char_index[end_byte];

        // Determine which pattern matched by getting the pattern ID and map via counts
        let pattern_id = mat.pattern().as_usize();
        let total_patterns = level_metadata.enterprise_count + level_metadata.user_count;
        if pattern_id >= total_patterns {
            log::error!("Secret level not found for pattern ID {pattern_id}");
            continue;
        }
        let secret_level = if pattern_id < level_metadata.enterprise_count {
            SecretLevel::Enterprise
        } else {
            SecretLevel::User
        };

        secret_ranges.push((
            SecretRange {
                char_range: start_char..end_char,
                byte_range: start_byte..end_byte,
            },
            secret_level,
        ));
    }

    // Merge overlapping ranges, preserving the highest priority SecretLevel
    merge_sorted_ranges_with_levels(secret_ranges)
}

/// Merges overlapping ranges while preserving the highest priority SecretLevel
fn merge_sorted_ranges_with_levels(
    ranges: Vec<(SecretRange, SecretLevel)>,
) -> Vec<(SecretRange, SecretLevel)> {
    if ranges.is_empty() {
        return ranges;
    }

    let mut merged_ranges = vec![];
    let mut current_range = ranges[0].0.clone();
    let mut current_level = ranges[0].1;

    for (range, level) in ranges.into_iter().skip(1) {
        // We can merge based on character ranges since non-overlapping character ranges result in non-overlapping byte ranges.
        if range.char_range.start <= current_range.char_range.end {
            // Extend the current range to include the overlapping range.
            current_range.extend_range_end(&range);
            // Keep the highest priority level
            if level.priority() > current_level.priority() {
                current_level = level;
            }
        } else {
            // No overlap, push the current range and move to the next.
            merged_ranges.push((current_range, current_level));
            current_range = range;
            current_level = level;
        }
    }

    // Add the last range.
    merged_ranges.push((current_range, current_level));

    merged_ranges
}

impl GridHandler {
    pub fn num_secrets_obfuscated(&self) -> usize {
        self.secrets.len()
    }

    pub fn get_secret_obfuscation(&self) -> ObfuscateSecrets {
        self.secret_obfuscation_mode
    }

    /// Returns a tuple of [`Secret`] and [`SecretHandle`] at the given point (assuming the point is
    /// the displayed location and needs to be translated to the original location) or `None` if none is identified.
    pub fn secret_at_displayed_point(&self, displayed_point: Point) -> Option<SecretAndHandle<'_>> {
        self.secrets
            .get_by_point(displayed_point, self, RespectDisplayedOutput::Yes)
    }

    /// Returns a tuple of [`Secret`] and [`SecretHandle`] at the given point (assuming
    /// the point is the original location in the grid) or `None` if none is identified.
    pub fn secret_at_original_point(&self, original_point: Point) -> Option<SecretAndHandle<'_>> {
        self.secrets
            .get_by_point(original_point, self, RespectDisplayedOutput::No)
    }

    /// Returns a [`Secret`] identified by [`SecretHandle`] or `None` if none is identified.
    pub fn secret_by_handle(&self, secret_handle: SecretHandle) -> Option<&Secret> {
        self.secrets.get_by_handle(&secret_handle)
    }

    /// Finds all secrets with matching plaintext and updates whether or not they are obfuscated.
    fn mark_matching_secrets(&mut self, secret_handle: &SecretHandle, is_obfuscated: IsObfuscated) {
        let Some(secret) = self.secrets.get_by_handle(secret_handle) else {
            return;
        };

        let secret_plaintext = self.generate_secret_plaintext(secret.range());
        let Some(matching_secret_handles) =
            self.secrets_in_plaintext.get(secret_plaintext.as_str())
        else {
            return;
        };

        for secret_handle in matching_secret_handles.iter() {
            if let Err(e) = self.secrets.set_is_obfuscated(secret_handle, is_obfuscated) {
                log::warn!("Unable to obfuscate secret: {e:?}");
            }
        }
    }

    pub(in crate::terminal::model) fn obfuscate_secrets(
        &mut self,
        obfuscate_secrets: ObfuscateSecrets,
    ) {
        self.secret_obfuscation_mode = obfuscate_secrets;
    }

    /// Marks the secret identified by [`SecretHandle`] as obfuscated. Returns an `Err` if no secret
    /// is identified by [`SecretHandle`].
    pub fn obfuscate_secret(&mut self, secret_handle: SecretHandle) -> anyhow::Result<()> {
        self.secrets
            .set_is_obfuscated(&secret_handle, IsObfuscated::Yes)
    }

    /// Marks the secret identified by [`SecretHandle`] as unobfuscated. Returns an `Err` if no
    /// secret is identified by [`SecretHandle`].
    pub fn unobfuscate_secret(&mut self, secret_handle: SecretHandle) -> anyhow::Result<()> {
        self.secrets
            .set_is_obfuscated(&secret_handle, IsObfuscated::No)?;

        self.mark_matching_secrets(&secret_handle, IsObfuscated::No);
        Ok(())
    }

    /// Marks the range of points identified by `range` as a secret within the grid.
    pub(in crate::terminal::model::grid) fn mark_secret_range(
        &mut self,
        range: RangeInclusive<Point>,
        is_obfuscated: IsObfuscated,
        plaintext: String,
        secret_level: SecretLevel,
    ) {
        let handle = SecretHandle::next();

        let range = *range.start()..=*range.end();
        let secret = Secret::new(is_obfuscated, range.clone(), secret_level);
        self.secrets.insert(handle, secret, self.columns());

        self.secrets_in_plaintext
            .entry(plaintext)
            .or_default()
            .insert(handle);
    }

    /// Clears any secrets currently stored in the grid.
    pub(in crate::terminal::model) fn clear_secrets(&mut self) {
        self.secrets.clear();
        self.secrets_in_plaintext.clear();
        self.set_all_bytes_scanned_for_secrets(true);
    }

    /// Clears the secrets that are encompassed in the given range. Returns a range of the minimum
    /// and maximum point of all of the secrets that were removed.
    pub(super) fn clear_secrets_in_range(
        &mut self,
        range: RangeInclusive<Point>,
    ) -> Option<RangeInclusive<Point>> {
        // Determine the set of secrets that need to be removed and eagerly collect them into a vec.
        // We can't mutate the cells or the secrets map directly within the iterator due to lifetime
        // issues.
        let secrets_to_remove = self
            .secrets
            .iter()
            .filter_map(|(secret_handle, secret)| {
                let secret_range = secret.range();
                // If the secret intersects with the dirty range at all, clear it.
                let ranges_intersect =
                    range.start() <= secret_range.end() && range.end() >= secret_range.start();
                ranges_intersect.then_some((*secret_handle, secret_range))
            })
            .collect_vec();

        let (_, secret_range) = secrets_to_remove.first()?;
        let mut start_point = *secret_range.start();
        let mut end_point = *secret_range.end();

        for (secret_handle, secret_range) in secrets_to_remove {
            start_point = start_point.min(*secret_range.start());
            end_point = end_point.max(*secret_range.end());

            self.secrets.remove(secret_handle, self.columns());
        }

        Some(start_point..=end_point)
    }

    /// Scans the entire grid (not just the dirty cells range) for secrets.
    pub fn scan_full_grid_for_secrets(&mut self) {
        let start_point = Point::new(0, 0);
        let end_point = Point::new(self.total_rows(), self.columns());

        self.scan_range_for_secrets(start_point..=end_point);
        self.set_all_bytes_scanned_for_secrets(true);
    }

    /// Scans the grid for any secrets in the cells that are currently marked as "dirty".
    /// To scan for secrets, we take the beginning and end of the dirty cell range and expand this
    /// range to its word boundaries. We then invalidate any prior secret that is contained within
    /// this range, and then scan for secrets using regular expressions.
    /// Returns the number of secret matches found.
    fn scan_dirty_cells_for_secrets(&mut self) {
        let Some((dirty_range_start, dirty_range_end)) =
            self.dirty_cells_range().map(RangeInclusive::into_inner)
        else {
            return;
        };

        // Expand both the start and end points to word boundaries. The range of dirty cells is not
        // guaranteed to be at a word boundary, in which case we would incorrectly omit secrets that
        // start before the range or end after the range.
        let start_point = self
            .nonblank_word_bound_before_point(dirty_range_start)
            .unwrap_or(dirty_range_start);

        let end_point = self
            .nonblank_word_bound_after_point(dirty_range_end)
            .unwrap_or(dirty_range_end);

        self.scan_range_for_secrets(start_point..=end_point);
    }

    /// Rescans the entire grid for secrets after a resize.
    pub(super) fn scan_for_secrets_after_resize(&mut self) {
        // Clear the entire secret range map since the underlying ranges stored within the map may
        // be out of date after the resize.
        self.secrets.clear_ranges_after_resize();
        self.scan_range_for_secrets(
            Point::new(0, 0)..=Point::new(self.total_rows() - 1, self.columns() - 1),
        );
    }

    /// Scans the given `range` for secrets. Any prior secrets within the range are removed from the
    /// grid. Any match is visually redacted in the grid.
    fn scan_range_for_secrets(&mut self, range: RangeInclusive<Point>) {
        let old_unobfuscated_secrets = &self
            .secrets
            .iter()
            .filter_map(|(_, secret)| {
                if !secret.is_obfuscated() {
                    Some(self.generate_secret_plaintext(secret.range()))
                } else {
                    None
                }
            })
            .collect::<HashSet<String>>();

        // Clear any secrets that may are encompassed by the range that we are now searching --
        // they may no longer be valid.
        let cleared_secrets_range = self.clear_secrets_in_range(range.clone());

        let mut start_point = *range.start();
        let mut end_point = *range.end();

        // Adjust the start and end points of where we are scanning if we cleared any secrets that
        // started _before_ the range we are searching or ended _after_ the range we are searching.
        // If we don't expand the boundaries of where we are searching, we may end up incorrectly
        // removing a secret without refinding it.
        if let Some(cleared_secrets_range) = cleared_secrets_range {
            start_point = start_point.min(*cleared_secrets_range.start());
            end_point = end_point.max(*cleared_secrets_range.end());
        }

        let secrets_regex: Arc<SecretsRegex> = { SECRETS_REGEX.lock().clone() };
        let matches = self
            .regex_iter(
                start_point,
                end_point,
                Direction::Right,
                &secrets_regex.dfas,
            )
            .collect_vec();

        for secret_match in matches {
            // We mark a secret as unobfuscated if there is a new secret in the dirty cells range
            // that has the same plaintext as a previous secret that was unobfuscated.
            let plaintext = self.generate_secret_plaintext(secret_match.clone());
            let old_secret_found = old_unobfuscated_secrets.contains(plaintext.as_str());
            let is_obfuscated = if old_secret_found {
                IsObfuscated::No
            } else {
                IsObfuscated::Yes
            };

            // Determine the secret level by re-scanning the plaintext
            let secret_level = self.determine_secret_level(&plaintext, &secrets_regex);

            self.mark_secret_range(secret_match, is_obfuscated, plaintext, secret_level);
        }
    }

    fn generate_secret_plaintext(&self, range: RangeInclusive<Point>) -> String {
        let mut text = String::new();

        let mut cursor = self.grapheme_cursor_from(*range.start(), grapheme_cursor::Wrap::All);

        while let Some(item) = cursor.current_item() {
            if !((item.point().row < range.end().row)
                || (item.point().row <= range.end().row && item.point().col <= range.end().col))
            {
                break;
            }
            text.push(item.cell().c);
            cursor.move_forward();
        }
        text
    }

    /// Determines the secret level by re-scanning the plaintext using the rich content detection
    /// which includes secret level information
    fn determine_secret_level(&self, plaintext: &str, regex: &SecretsRegex) -> SecretLevel {
        let secrets_with_levels = find_secrets_in_text_with_levels_using_regex(plaintext, regex);

        // Find the first match that corresponds to our plaintext
        // In case of multiple matches, we return the highest priority level
        secrets_with_levels
            .into_iter()
            .map(|(_, level)| level)
            .max_by_key(|level| level.priority())
            .unwrap_or(SecretLevel::User) // Default to User level if no matches found
    }

    fn set_all_bytes_scanned_for_secrets(&mut self, value: bool) {
        self.all_bytes_scanned_for_secrets = value;
    }

    pub(in crate::terminal::model) fn all_bytes_scanned_for_secrets(&self) -> bool {
        self.all_bytes_scanned_for_secrets
    }

    pub(super) fn maybe_scan_dirty_cells_for_secrets(&mut self) {
        if self.secret_obfuscation_mode.should_redact_secret() {
            self.scan_dirty_cells_for_secrets();
            self.all_bytes_scanned_for_secrets &= true;
        } else {
            self.all_bytes_scanned_for_secrets = false;
        }
    }
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod tests;
