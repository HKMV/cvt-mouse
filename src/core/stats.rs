//! 运行统计：界面每 250ms 读一次的那几个数。
//!
//! 和 [`crate::core::policy`] 分开放，是因为它只是**观测数据**：谁写谁读都行，
//! 决策本身不需要它。放在一起会让「要不要改这一格」和「界面上显示什么」纠缠。

/// 运行统计。
///
/// 计数用 **wheel 单位**（120 = 一格）而不是格：鼠标送来的值不一定是 120 的整数倍，
/// 按格记会把细粒度事件记成 0，看起来像「什么都没收到」。
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// 经过钩子的滚轮事件数。
    pub events: u64,
    /// 累计进来的 wheel 单位（绝对值）。
    pub in_units: i64,
    /// 累计发给应用的 wheel 单位（绝对值）。
    pub out_units: i64,
    /// 原样放行的次数。
    pub passed: u64,
    /// 吃掉但没发的次数。
    pub swallowed: u64,
    /// 最近一次的速度（格/秒）。
    pub velocity: f32,
    /// 最近一次的倍率。
    pub gain: f32,
    /// 最近一次收到的原始 `mouseData`（±120 才是标准一格）。
    pub last_delta: i32,
    /// 最近一次事件的 `dwExtraInfo`：用来认出「这条是谁注入的」。
    pub last_extra: usize,
    /// 最近一次事件的 `dwFlags`。
    pub last_flags: u32,
    /// 最近一次算倍率时认到的程序。
    pub exe: String,
}
