//! 几个纯格式化函数。颜色/圆角/间距一律交给 nsh 的主题 token 和 Tailwind
//! class（`bg-card`、`text-muted-foreground`、`rounded-(--radius)`），这里不放
//! 任何色值。

/** 倍率 → `2.5x`。 */
export function gainText(gain: number): string {
  return `${gain.toFixed(1)}x`;
}

/** 速度 → `12.0 格/秒`。 */
export function speedText(velocity: number): string {
  return `${velocity.toFixed(1)} 格/秒`;
}
