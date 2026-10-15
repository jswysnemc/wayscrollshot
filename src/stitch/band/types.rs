pub const NO_MATCH: f32 = 1.0e9;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Edge {
    #[default]
    None,
    Start,
    End,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    FirstFrame,
    Appended,
    NoProgress,
    #[default]
    NoMatch,
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub min_overlap: i32,
    pub accept_diff: f32,
    pub min_append: i32,
    pub approx_diff: f32,
}

impl Default for Config {
    /// 返回 mark-shot 使用的生产拼接阈值；无参数。
    fn default() -> Self {
        Self {
            min_overlap: 100,
            accept_diff: 9.0,
            min_append: 15,
            approx_diff: 1.0,
        }
    }
}

impl Config {
    /// 校验重叠、步长与置信阈值；无参数，返回配置是否安全。
    pub fn valid(self) -> bool {
        self.min_overlap > 0
            && self.min_append > 0
            && self.min_overlap <= crate::stitch::band::pixels::MAX_PIXELS as i32
            && self.min_append <= crate::stitch::band::pixels::MAX_PIXELS as i32
            && self.accept_diff.is_finite()
            && self.accept_diff >= 0.0
            && self.accept_diff < NO_MATCH
            && self.approx_diff.is_finite()
            && self.approx_diff >= 0.0
            && self.approx_diff <= self.accept_diff
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StitchResult {
    pub status: Status,
    pub added: i32,
    pub edge: Edge,
    pub position: i32,
    pub frame_length: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub frame_count: i32,
    pub total_height: i32,
    pub last_append: i32,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LineMatch {
    pub position: i32,
    pub diff: f32,
    pub trim: i32,
    pub rows: i32,
    pub edge: Edge,
}

impl Default for LineMatch {
    /// 返回无匹配的行运行候选；无参数。
    fn default() -> Self {
        Self {
            position: 0,
            diff: NO_MATCH,
            trim: 0,
            rows: 0,
            edge: Edge::None,
        }
    }
}

/// 计算 position 处帧相对 full_height 的越界量；返回新增长度和生长方向。
pub(crate) fn overhang(position: i32, frame_height: i32, full_height: i32) -> (i32, Edge) {
    let top = (-position).max(0);
    let bottom = (position + frame_height - full_height).max(0);
    (
        top.max(bottom),
        if bottom >= top {
            Edge::End
        } else {
            Edge::Start
        },
    )
}
