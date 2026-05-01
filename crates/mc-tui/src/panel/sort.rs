use std::cmp::Ordering;

use mc_core::Entry;
use mc_core::action::SortKey;

pub fn sort_entries(entries: &mut [Entry], key: SortKey, reverse: bool, mix_dirs: bool) {
    entries.sort_by(|a, b| {
        // Always keep ".." pinned at the top regardless of sort.
        if a.name == ".." && b.name != ".." {
            return Ordering::Less;
        }
        if b.name == ".." && a.name != ".." {
            return Ordering::Greater;
        }

        if !mix_dirs {
            match (a.is_dir(), b.is_dir()) {
                (true, false) => return Ordering::Less,
                (false, true) => return Ordering::Greater,
                _ => {}
            }
        }

        let ord = match key {
            SortKey::Unsorted => Ordering::Equal,
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Extension => extension_of(&a.name)
                .cmp(extension_of(&b.name))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Mtime => a.mtime.cmp(&b.mtime),
            SortKey::Atime => a.atime.cmp(&b.atime),
            SortKey::Ctime => a.ctime.cmp(&b.ctime),
            SortKey::Inode => Ordering::Equal,
        };
        if reverse { ord.reverse() } else { ord }
    });
}

fn extension_of(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(_, ext)| ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_core::EntryKind;

    fn entry(name: &str, kind: EntryKind, size: u64) -> Entry {
        Entry {
            name: name.into(),
            kind,
            size,
            mtime: None,
            atime: None,
            ctime: None,
            mode: None,
            uid: None,
            gid: None,
            nlink: None,
            target: None,
        }
    }

    #[test]
    fn dirs_first_by_default() {
        let mut v = vec![
            entry("apple.txt", EntryKind::File, 1),
            entry("zoo", EntryKind::Dir, 0),
            entry("banana.txt", EntryKind::File, 2),
        ];
        sort_entries(&mut v, SortKey::Name, false, false);
        assert_eq!(v[0].name, "zoo");
        assert_eq!(v[1].name, "apple.txt");
    }

    #[test]
    fn parent_pinned_at_top() {
        let mut v = vec![
            entry("apple.txt", EntryKind::File, 1),
            entry("..", EntryKind::Dir, 0),
            entry("zoo", EntryKind::Dir, 0),
        ];
        sort_entries(&mut v, SortKey::Name, true, false);
        assert_eq!(v[0].name, "..");
    }

    #[test]
    fn reverse_swaps_order() {
        let mut v = vec![
            entry("a", EntryKind::File, 1),
            entry("b", EntryKind::File, 2),
        ];
        sort_entries(&mut v, SortKey::Name, true, true);
        assert_eq!(v[0].name, "b");
    }

    #[test]
    fn extension_sort() {
        let mut v = vec![
            entry("a.zzz", EntryKind::File, 0),
            entry("b.aaa", EntryKind::File, 0),
        ];
        sort_entries(&mut v, SortKey::Extension, false, true);
        assert_eq!(v[0].name, "b.aaa");
    }
}
