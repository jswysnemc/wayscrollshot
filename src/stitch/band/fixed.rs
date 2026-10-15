use super::pixels::Image;
use super::profile::{row_diff, side_ignore, ROW_MAX_DIFF};

#[derive(Clone, Copy, Debug, Default)]
pub struct Observation {
    pub header: i32,
    pub footer: i32,
    pub valid: bool,
}

#[derive(Clone, Default)]
struct Band {
    committed: i32,
    pending: Option<i32>,
    sampled: bool,
}

impl Band {
    /// 用 candidate 更新固定带；缩小立即生效，扩大需连续两次，返回值为空。
    fn update(&mut self, candidate: i32) {
        if !self.sampled || candidate <= self.committed {
            self.committed = candidate;
            self.sampled = true;
            self.pending = None;
        } else if let Some(pending) = self.pending {
            self.committed = pending.min(candidate);
            self.pending = None;
        } else {
            self.pending = Some(candidate);
        }
    }
}

#[derive(Clone, Default)]
pub struct FixedRegions {
    header: Band,
    footer: Band,
}

impl FixedRegions {
    /// 清除全部观察状态；无参数、无返回值。
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// 返回稳定顶部固定带高度；无参数。
    pub fn top(&self) -> i32 {
        self.header.committed
    }

    /// 返回稳定底部固定带高度；无参数。
    pub fn bottom(&self) -> i32 {
        self.footer.committed
    }

    /// 【长截图】【固定栏检测】比较 previous、current，offset 为可信位移；返回本次候选并更新平滑状态。
    pub fn observe(&mut self, previous: &Image, current: &Image, offset: i32) -> Observation {
        if offset.unsigned_abs() < 12
            || previous.width() != current.width()
            || previous.height() != current.height()
        {
            return Observation::default();
        }
        let header = scan(previous, current, false);
        let footer = scan(previous, current, true);
        self.header.update(header);
        self.footer.update(footer);
        Observation {
            header,
            footer,
            valid: true,
        }
    }
}

/// 比较 previous、current 从指定端起的相同行；bottom 指定底端，返回固定行数。
fn scan(previous: &Image, current: &Image, bottom: bool) -> i32 {
    let side = side_ignore(current.width());
    let width = current.width() - side * 2;
    let mut count = 0;
    while count < current.height() / 3 {
        let y = if bottom {
            current.height() - 1 - count
        } else {
            count
        };
        if row_diff(previous.row(y), current.row(y), side, width) > ROW_MAX_DIFF {
            break;
        }
        count += 1;
    }
    count
}
