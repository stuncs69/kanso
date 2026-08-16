use crate::buffer::Position;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOp {
    Insert { at: usize, text: String },
    Delete { at: usize, text: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    Insert,
    Delete,
}

#[derive(Debug, Clone, Copy)]
pub enum Merge {
    Start,
    Always,
    Auto(GroupKind),
}

#[derive(Debug)]
pub struct Transaction {
    pub ops: Vec<EditOp>,
    pub before: Position,
    pub after: Position,
}

#[derive(Default)]
pub struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    pending: Option<(Transaction, Option<GroupKind>)>,
}

impl History {
    pub fn record(&mut self, op: EditOp, merge: Merge, before: Position, after: Position) {
        self.redo.clear();
        let can_merge = match (&merge, &self.pending) {
            (Merge::Always, Some(_)) => true,
            (Merge::Auto(kind), Some((tx, Some(pending_kind)))) if pending_kind == kind => {
                auto_mergeable(tx.ops.last(), &op, *kind)
            }
            _ => false,
        };
        if can_merge {
            let (tx, _) = self.pending.as_mut().expect("checked above");
            tx.ops.push(op);
            tx.after = after;
        } else {
            self.commit();
            let kind = match merge {
                Merge::Auto(kind) => Some(kind),
                _ => None,
            };
            self.pending = Some((
                Transaction {
                    ops: vec![op],
                    before,
                    after,
                },
                kind,
            ));
        }
    }

    pub fn commit(&mut self) {
        if let Some((tx, _)) = self.pending.take() {
            self.undo.push(tx);
        }
    }

    pub fn pop_undo(&mut self) -> Option<Transaction> {
        self.commit();
        self.undo.pop()
    }

    pub fn pop_redo(&mut self) -> Option<Transaction> {
        self.redo.pop()
    }

    pub fn push_redo(&mut self, tx: Transaction) {
        self.redo.push(tx);
    }

    pub fn push_undo(&mut self, tx: Transaction) {
        self.undo.push(tx);
    }
}

fn auto_mergeable(last: Option<&EditOp>, new: &EditOp, kind: GroupKind) -> bool {
    let Some(last) = last else {
        return false;
    };
    match (kind, last, new) {
        (
            GroupKind::Insert,
            EditOp::Insert { at, text },
            EditOp::Insert {
                at: new_at,
                text: new_text,
            },
        ) => !new_text.contains('\n') && *new_at == at + text.chars().count(),
        (
            GroupKind::Delete,
            EditOp::Delete { at, .. },
            EditOp::Delete {
                at: new_at,
                text: new_text,
            },
        ) => new_at + new_text.chars().count() == *at || new_at == at,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(at: usize, text: &str) -> EditOp {
        EditOp::Insert {
            at,
            text: text.to_string(),
        }
    }

    #[test]
    fn contiguous_inserts_merge() {
        let mut h = History::default();
        let p = Position::default();
        h.record(insert(0, "h"), Merge::Auto(GroupKind::Insert), p, p);
        h.record(insert(1, "i"), Merge::Auto(GroupKind::Insert), p, p);
        let tx = h.pop_undo().unwrap();
        assert_eq!(tx.ops.len(), 2);
        assert!(h.pop_undo().is_none());
    }

    #[test]
    fn non_contiguous_inserts_do_not_merge() {
        let mut h = History::default();
        let p = Position::default();
        h.record(insert(0, "h"), Merge::Auto(GroupKind::Insert), p, p);
        h.record(insert(5, "i"), Merge::Auto(GroupKind::Insert), p, p);
        assert_eq!(h.pop_undo().unwrap().ops.len(), 1);
        assert_eq!(h.pop_undo().unwrap().ops.len(), 1);
    }

    #[test]
    fn start_policy_always_splits() {
        let mut h = History::default();
        let p = Position::default();
        h.record(insert(0, "h"), Merge::Auto(GroupKind::Insert), p, p);
        h.record(insert(1, "i"), Merge::Start, p, p);
        assert_eq!(h.pop_undo().unwrap().ops.len(), 1);
        assert_eq!(h.pop_undo().unwrap().ops.len(), 1);
    }

    #[test]
    fn always_policy_fuses_composites() {
        let mut h = History::default();
        let p = Position::default();
        h.record(
            EditOp::Delete {
                at: 0,
                text: "sel".to_string(),
            },
            Merge::Start,
            p,
            p,
        );
        h.record(insert(0, "x"), Merge::Always, p, p);
        assert_eq!(h.pop_undo().unwrap().ops.len(), 2);
    }

    #[test]
    fn recording_clears_redo() {
        let mut h = History::default();
        let p = Position::default();
        h.record(insert(0, "a"), Merge::Start, p, p);
        let tx = h.pop_undo().unwrap();
        h.push_redo(tx);
        h.record(insert(0, "b"), Merge::Start, p, p);
        assert!(h.pop_redo().is_none());
    }
}
