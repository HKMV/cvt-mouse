//! 版式小块：卡片、一行设置、说明、读数。
//!
//! 这里**只有布局**，没有控件、没有色值：按钮用 `nsh/base` 的 `Button`、开关用
//! `Switch`、数值用 `Slider`，外观靠 Tailwind class 与主题 token（`bg-card`、
//! `text-muted-foreground`、`rounded-(--radius)` …）。卡片在 base 里没有对应
//! 组件，所以用 class 拼一个。

import { Text, View } from "nsh";
import type { JSX } from "nsh";

/** 一张卡片：底色 + 描边 + 主题圆角。 */
export function Card(props: { title: string; hint?: string; right?: JSX.Element; children?: JSX.Element }): JSX.Element {
  return (
    <View class="flex-col gap-3 rounded-(--radius) border border-border bg-card p-4">
      <View class="flex-row items-center gap-3">
        <View class="flex-1 flex-col gap-1">
          <Text class="text-sm font-semibold text-card-foreground">{props.title}</Text>
          {props.hint ? <Text class="text-xs text-muted-foreground">{props.hint}</Text> : null}
        </View>
        {props.right}
      </View>
      {props.children}
    </View>
  );
}

/** 一行设置：左边标题与说明，右边控件。 */
export function Row(props: { label: string; hint?: string; children: JSX.Element }): JSX.Element {
  return (
    <View class="flex-row items-center gap-3">
      <View class="flex-1 flex-col gap-1">
        <Text class="text-xs font-medium text-foreground">{props.label}</Text>
        {props.hint ? <Text class="text-xs text-muted-foreground">{props.hint}</Text> : null}
      </View>
      {props.children}
    </View>
  );
}

/** 一句说明文字。 */
export function Caption(props: { children: string }): JSX.Element {
  return <Text class="text-xs text-muted-foreground">{props.children}</Text>;
}

/** 一句结论（成功绿、失败红）。 */
export function Note(props: { ok: boolean; text: string }): JSX.Element {
  return <Text class={props.ok ? "text-xs text-primary" : "text-xs text-destructive"}>{props.text}</Text>;
}

/** 一组「标题 + 数字」。`tone` 只给需要报警的读数用（比如注入失败 > 0）。 */
export function Metric(props: { label: string; value: string; tone?: "normal" | "danger" }): JSX.Element {
  return (
    <View class="flex-1 flex-col gap-1">
      <Text class="text-xs text-muted-foreground">{props.label}</Text>
      <Text
        class={
          props.tone === "danger"
            ? "text-sm font-semibold text-destructive"
            : "text-sm font-semibold text-foreground"
        }
      >
        {props.value}
      </Text>
    </View>
  );
}

/** 控件左边的一格读数：宽度固定，数字变了不会让布局跳。 */
export function Value(props: { text: string }): JSX.Element {
  return <Text class="w-16 text-right text-xs font-semibold text-foreground">{props.text}</Text>;
}
