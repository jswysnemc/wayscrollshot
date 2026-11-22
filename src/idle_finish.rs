use std::time::{Duration, Instant};

/// 空闲自动完成控制器，用于在无滚动更新达到阈值后触发自动保存或复制
pub struct IdleAutoFinish {
    timeout: Option<Duration>,
    has_captured_frame: bool,
    last_activity_at: Option<Instant>,
    triggered: bool,
}

impl IdleAutoFinish {
    /// 创建空闲自动完成控制器
    ///
    /// `timeout` 为允许的最大空闲时长，`None` 表示不启用自动完成；返回控制器实例
    pub fn new(timeout: Option<Duration>) -> Self {
        Self {
            timeout,
            has_captured_frame: false,
            last_activity_at: None,
            triggered: false,
        }
    }

    /// 记录首帧捕获完成并启动空闲计时
    ///
    /// `now` 为当前时间戳；无返回值
    pub fn on_first_frame(&mut self, now: Instant) {
        self.has_captured_frame = true;
        self.last_activity_at = Some(now);
    }

    /// 记录成功追加新内容并重置空闲计时
    ///
    /// `now` 为当前时间戳；无返回值
    pub fn on_appended(&mut self, now: Instant) {
        self.has_captured_frame = true;
        self.last_activity_at = Some(now);
    }

    /// 暂停捕获时挂起空闲计时，避免暂停时间被计入空闲时长
    ///
    /// 该方法无传参和返回值
    pub fn on_paused(&mut self) {
        self.last_activity_at = None;
    }

    /// 检查当前是否已满足空闲自动完成条件
    ///
    /// `now` 为当前时间戳，`paused` 表示当前是否处于暂停状态；返回是否应当触发自动完成（最多触发一次）
    pub fn should_trigger(&mut self, now: Instant, paused: bool) -> bool {
        // 1. 未启用、已触发、处于暂停或尚未捕获首帧时不触发
        let Some(timeout) = self.timeout else {
            return false;
        };
        if self.triggered || paused || !self.has_captured_frame {
            return false;
        }

        // 2. 从暂停恢复后以当前时刻重新起算空闲时长
        let last_activity = match self.last_activity_at {
            Some(ts) => ts,
            None => {
                self.last_activity_at = Some(now);
                return false;
            }
        };

        // 3. 达到空闲阈值后标记已触发并返回
        if now.saturating_duration_since(last_activity) >= timeout {
            self.triggered = true;
            return true;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::IdleAutoFinish;

    #[test]
    fn disabled_never_triggers() {
        let now = Instant::now();
        let mut finish = IdleAutoFinish::new(None);
        finish.on_first_frame(now);

        assert!(!finish.should_trigger(now + Duration::from_secs(60), false));
    }

    #[test]
    fn does_not_trigger_before_first_frame() {
        let now = Instant::now();
        let mut finish = IdleAutoFinish::new(Some(Duration::from_secs(2)));

        assert!(!finish.should_trigger(now + Duration::from_secs(10), false));
    }

    #[test]
    fn triggers_once_after_idle_timeout() {
        let now = Instant::now();
        let mut finish = IdleAutoFinish::new(Some(Duration::from_secs(2)));
        finish.on_first_frame(now);

        assert!(!finish.should_trigger(now + Duration::from_millis(1999), false));
        assert!(finish.should_trigger(now + Duration::from_secs(2), false));
        assert!(!finish.should_trigger(now + Duration::from_secs(5), false));
    }

    #[test]
    fn append_resets_idle_timer() {
        let now = Instant::now();
        let mut finish = IdleAutoFinish::new(Some(Duration::from_secs(2)));
        finish.on_first_frame(now);
        finish.on_appended(now + Duration::from_millis(1500));

        assert!(!finish.should_trigger(now + Duration::from_millis(3000), false));
        assert!(finish.should_trigger(now + Duration::from_millis(3500), false));
    }

    #[test]
    fn pause_suspends_and_restarts_idle_timer() {
        let now = Instant::now();
        let mut finish = IdleAutoFinish::new(Some(Duration::from_secs(2)));
        finish.on_first_frame(now);
        finish.on_paused();

        assert!(!finish.should_trigger(now + Duration::from_secs(10), true));
        assert!(!finish.should_trigger(now + Duration::from_secs(10), false));
        assert!(!finish.should_trigger(now + Duration::from_secs(11), false));
        assert!(finish.should_trigger(now + Duration::from_secs(12), false));
    }
}
