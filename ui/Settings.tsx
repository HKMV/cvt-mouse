//! 主界面：一页设置，从上到下是曲线、按应用、模式与透传、统计、日志。
//!
//! 控件全部来自 `nsh/base`（`Button` / `Switch` / `Slider` / `NumberInput`），
//! 外观靠 Tailwind class 与主题 token（`bg-card`、`text-muted-foreground`、
//! `rounded-(--radius)`），本仓库不定义任何色值。
//!
//! # nsh 的两条规矩（踩过的坑，别改回去）
//!
//! 1. **组件体只跑一次**。跟着信号变的东西都要写在 JSX 里（`{snap()?.stats…}`），
//!    写成组件体里的常量会冻在首帧。
//! 2. 列表用 `.map()` 直接写在子节点位置；nsh 没有 React 的 `key`。

import { Scroll, signal, Text, View } from "nsh";
import type { JSX } from "nsh";
import { Button, Collapsible, NumberInput, Slider, Switch } from "nsh/base";
import { Area, Grid, Line, Plot, PlotAxis } from "nsh/base/plot";
import * as api from "./lib/api";
import { Caption, Card, Metric, Note, Row, Value } from "./components/ui";
import { bindWindow, installTray, interceptClose } from "./lib/desktop";
import { gainText, speedText } from "./format";

// ---- class 常量 ----------------------------------------------------------

const BTN = "h-8 rounded-full px-3";
const BTN_GHOST = `${BTN} border border-border hover:bg-accent`;
const BTN_DANGER = `${BTN} border border-destructive hover:bg-accent`;
/** 分段控件的两半：选中那半填色，另一半只留 hover。 */
const SEG_ON = "h-7 rounded-full px-3 bg-primary";
const SEG_OFF = "h-7 rounded-full px-3 hover:bg-accent";
const LABEL = "text-xs text-foreground";
const LABEL_PRIMARY = "text-xs text-primary-foreground";
const LABEL_DANGER = "text-xs text-destructive";
const SLIDER = "w-56";
const SLIDER_TRACK = "h-6 justify-center";
const SLIDER_INDICATOR = "h-1.5 rounded-full bg-muted";
const SLIDER_RANGE = "rounded-full bg-primary";
const SLIDER_THUMB = "size-4 rounded-full border border-border bg-background";
const SWITCH_TRACK = "h-5 w-9 rounded-full bg-muted checked:bg-primary";
const SWITCH_THUMB = "size-4 rounded-full bg-background checked:translate-x-4";

// ---- 状态 ----------------------------------------------------------------

const boot = api.boot();
const meta = boot.meta;
const config = signal<api.Config>(boot.config);
const snap = signal<api.Snapshot | null>(null);
const message = signal<{ ok: boolean; text: string } | null>(null);
const tested = signal<api.SelfTest | null>(null);
/** 顶部统计条是展开还是收起。 */
const statsOpen = signal(false);

/** 曲线三个旋钮的草稿：拖的时候只动它，松手才落盘（拖一次会来几十个 onChange）。 */
const curve = signal({
  start: curveStart(),
  mid: curveMid(),
  high: curveHigh(),
});

// 250ms 一拍：够快看出「当前速度/倍率」在变，又不至于每秒重建几十次界面。
setInterval(() => {
  try {
    snap.set(api.snapshot());
  } catch {
    // 读快照失败不该让界面崩，下一拍再试。
  }
}, 250);

// ---- 配置 ----------------------------------------------------------------

function applyConfig(next: api.Config): void {
  const result = api.save(next);
  if (result.ok) {
    config.set(next);
    message.set({ ok: true, text: "已保存并生效" });
  } else {
    message.set({ ok: false, text: result.error ?? "保存失败" });
  }
}

function toggleEnabled(): void {
  setEnabled(!config().enabled);
}

function setEnabled(on: boolean): void {
  const result = api.setEnabled(on);
  if (!result.ok) {
    message.set({ ok: false, text: result.error ?? "切换失败" });
    return;
  }
  config.set({ ...config(), enabled: on });
  message.set({ ok: true, text: on ? "变速已开启" : "变速已关闭（滚轮恢复原生）" });
}

function curvePoints(): api.CurvePoint[] {
  const points = config().curve;
  if (points.length >= 4) return points;
  return [
    [0, 1],
    [6, 1],
    [12, 2.5],
    [24, 5],
  ];
}

function curveStart(): number {
  return curvePoints()[1][0];
}

function curveMid(): number {
  return curvePoints()[2][1];
}

function curveHigh(): number {
  return curvePoints()[3][1];
}

/** 滑块草稿对应的四个断点；`saveCurve` 与预览共用，免得两处漂移。 */
function draftPoints(): api.CurvePoint[] {
  const draft = curve();
  return [
    [0, 1],
    [draft.start, 1],
    [12, draft.mid],
    [24, draft.high],
  ];
}

/** 三个旋钮对应的就是这四个断点：`[0,1] [起点,1] [12,中速] [24,高速]`。 */
function saveCurve(): void {
  applyConfig({ ...config(), curve: draftPoints() });
}

/** 和 `src/core/curve.rs` 的 `Curve::gain` 同一套算法，只为画预览。 */
function gainAt(velocity: number): number {
  const points = draftPoints();
  if (velocity <= points[0][0]) return points[0][1];
  for (let i = 1; i < points.length; i += 1) {
    const [x0, g0] = points[i - 1];
    const [x1, g1] = points[i];
    if (velocity <= x1) {
      return x0 === x1 ? g1 : g0 + ((g1 - g0) * (velocity - x0)) / (x1 - x0);
    }
  }
  return points[points.length - 1][1];
}

/** 滑块的 `value` 既可能是单值也可能是区间，这里只要单值。 */
function single(value: number | [number, number]): number {
  return typeof value === "number" ? value : value[0];
}

// ---- 动作 ----------------------------------------------------------------

function addCurrentApp(): void {
  const exe = api.foreground().exe;
  if (!exe) {
    message.set({ ok: false, text: "认不出光标下的程序：把鼠标放到目标窗口上再点一次" });
    return;
  }
  const apps = { ...config().apps };
  if (apps[exe] === undefined) apps[exe] = 1;
  applyConfig({ ...config(), apps });
}

function removeApp(exe: string): void {
  const apps = { ...config().apps };
  delete apps[exe];
  applyConfig({ ...config(), apps });
}

function setAppGain(exe: string, gain: number): void {
  applyConfig({ ...config(), apps: { ...config().apps, [exe]: gain } });
}

function addCurrentSkip(): void {
  const exe = api.foreground().exe;
  if (!exe) {
    message.set({ ok: false, text: "认不出光标下的程序" });
    return;
  }
  if (config().skip.includes(exe)) return;
  applyConfig({ ...config(), skip: [...config().skip, exe] });
}

function removeSkip(exe: string): void {
  applyConfig({ ...config(), skip: config().skip.filter((item) => item !== exe) });
}

function runSelfTest(): void {
  const result = api.selfTest();
  tested.set(result);
  // 明细在统计条里，自检完直接摊开，不用再点一次。
  statsOpen.set(true);
  message.set({
    ok: true,
    text: `自检：连滚 ${result.burst.detents} 格（每 ${result.burst.intervalMs}ms 一格）→ 输出 ${result.burst.out} 格`,
  });
}

function relaunchElevated(): void {
  const result = api.relaunchElevated();
  if (!result.ok) message.set({ ok: false, text: result.error ?? "提权失败" });
}

// ---- 视图 ----------------------------------------------------------------

function statusText(): string {
  const state = snap();
  if (!state) return "正在读取状态…";
  if (!state.installed) return "钩子没装上：滚轮不会变速，看下面的日志";
  if (!state.enabled) return "已关闭：滚轮是系统原生手感";
  return `变速中 · ${state.exe || "（未识别）"} · ${gainText(state.stats.gain)} · ${speedText(state.stats.velocity)}`;
}

function statusDot(): string {
  const state = snap();
  if (!state) return "size-2 rounded-full bg-muted-foreground";
  if (!state.installed) return "size-2 rounded-full bg-destructive";
  if (!state.enabled) return "size-2 rounded-full bg-muted-foreground";
  return "size-2 rounded-full bg-primary";
}

function Header(): JSX.Element {
  return (
    <View class="flex-row items-center gap-3 border-b border-border bg-card px-4 py-2">
      <View class={statusDot()} />
      <View class="flex-1 flex-col">
        <Text class="text-base font-semibold text-foreground">CvtMouse 滚轮变速</Text>
        <Text class="text-xs text-muted-foreground">{statusText()}</Text>
      </View>
      <Button class={BTN_GHOST} onPress={runSelfTest}>
        <Text class={LABEL}>自检</Text>
      </Button>
      <Switch
        checked={config().enabled}
        onChange={(event) => setEnabled(event.checked)}
        trackClass={SWITCH_TRACK}
        thumbClass={SWITCH_THUMB}
      />
    </View>
  );
}

/** 预览的横轴：1 到 24 格/秒，一个速度一行。 */
const CURVE_TICKS = Array.from({ length: 24 }, (_, index) => index + 1);

/**
 * 曲线行：倍率在渲染时算进行里。
 *
 * `Plot` 只在 `data` 变了才重读各行的值，传一个常量数组会让预览冻住。
 */
function curveRows(): { v: number; g: number }[] {
  return CURVE_TICKS.map((v) => ({ v, g: gainAt(v) }));
}

/**
 * 曲线预览：nsh/base 的 `Plot`，横轴速度、纵轴倍率，虚线标出加速起点。
 *
 * 曲线本身是分段直线，所以 `curve` 用 `linear`（不是默认的平滑曲线）。
 */
function CurvePreview(): JSX.Element {
  return (
    <View class="flex-col gap-2">
      {/* pb-6 是给 x 轴刻度留的位置：`PlotAxis` 的标签画在图外面。 */}
      <View class="pb-6">
        <Plot
          class="h-28 w-full text-primary"
          data={curveRows()}
          x={(row) => row.v}
          y={(row) => row.g}
          xScale={{ type: "linear", domain: [1, 24] }}
          yScale={{ type: "linear", domain: [1, Math.max(curve().high, 1)] }}
          interactive={false}
        >
          <Grid class="stroke-border" />
          <Grid x={[curve().start]} class="stroke-muted-foreground" style={{ strokeDasharray: "4 4" }} />
          <Area curve="linear" class="fill-primary/10" />
          <Line curve="linear" class="stroke-primary" style={{ strokeWidth: 2 }} />
          <PlotAxis placement="bottom" ticks={4} labelClass="text-xs text-muted-foreground" />
          <PlotAxis
            placement="left"
            ticks={3}
            format={(value) => `${value}x`}
            labelClass="text-xs text-muted-foreground"
          />
        </Plot>
      </View>
      <Caption>横轴是滚动速度（格/秒），纵轴是倍率；虚线是加速起点，它左边保持原生手感。</Caption>
    </View>
  );
}

function CurveCard(): JSX.Element {
  return (
    <Card title="变速曲线" hint="滚得越快，一格顶的行数越多；一格一格滚永远不加速">
      <Row label="加速起点" hint="低于这个速度保持原生手感">
        <View class="flex-row items-center gap-3">
          <Value text={`${curve().start} 格/秒`} />
          <Slider
            class={SLIDER}
            testID="curve-start"
            value={curve().start}
            min={1}
            max={20}
            step={1}
            onChange={(event) => curve.update((draft) => ({ ...draft, start: single(event.value) }))}
            onChangeEnd={saveCurve}
            trackClass={SLIDER_TRACK}
            indicatorClass={SLIDER_INDICATOR}
            rangeClass={SLIDER_RANGE}
            thumbClass={SLIDER_THUMB}
          />
        </View>
      </Row>
      <Row label="中速倍率" hint="12 格/秒 时一格顶几行">
        <View class="flex-row items-center gap-3">
          <Value text={gainText(curve().mid)} />
          <Slider
            class={SLIDER}
            testID="curve-mid"
            value={curve().mid}
            min={1}
            max={8}
            step={0.1}
            onChange={(event) => curve.update((draft) => ({ ...draft, mid: single(event.value) }))}
            onChangeEnd={saveCurve}
            trackClass={SLIDER_TRACK}
            indicatorClass={SLIDER_INDICATOR}
            rangeClass={SLIDER_RANGE}
            thumbClass={SLIDER_THUMB}
          />
        </View>
      </Row>
      <Row label="高速倍率" hint="24 格/秒 及以上，封顶">
        <View class="flex-row items-center gap-3">
          <Value text={gainText(curve().high)} />
          <Slider
            class={SLIDER}
            testID="curve-high"
            value={curve().high}
            min={1}
            max={12}
            step={0.1}
            onChange={(event) => curve.update((draft) => ({ ...draft, high: single(event.value) }))}
            onChangeEnd={saveCurve}
            trackClass={SLIDER_TRACK}
            indicatorClass={SLIDER_INDICATOR}
            rangeClass={SLIDER_RANGE}
            thumbClass={SLIDER_THUMB}
          />
        </View>
      </Row>
      <CurvePreview />
    </Card>
  );
}

function AppsCard(): JSX.Element {
  return (
    <Card
      title="按应用"
      hint="同一套曲线下，给某个程序单独乘一个倍率（0.1 到 20）"
      right={
        <Button class={BTN_GHOST} onPress={addCurrentApp}>
          <Text class={LABEL}>加入光标下的程序</Text>
        </Button>
      }
    >
      {Object.keys(config().apps).length === 0 ? (
        <Caption>还没有单独配置的程序。把鼠标放到目标窗口上，点右上角「加入光标下的程序」。</Caption>
      ) : (
        Object.entries(config().apps).map(([exe, gain]) => (
          <View class="flex-row items-center gap-3">
            <Text class="flex-1 text-xs text-foreground">{exe}</Text>
            <NumberInput
              class="h-8 w-24 rounded-(--radius) border border-input bg-background px-2"
              value={gain}
              min={0.1}
              max={20}
              step={0.1}
              onChange={(event) => {
                if (event.value !== null) setAppGain(exe, event.value);
              }}
            />
            <Button class={BTN_DANGER} onPress={() => removeApp(exe)}>
              <Text class={LABEL_DANGER}>删除</Text>
            </Button>
          </View>
        ))
      )}
      <Row label="跳过列表" hint="这些程序完全不管，滚轮保持原生">
        <Button class={BTN_GHOST} onPress={addCurrentSkip}>
          <Text class={LABEL}>加入跳过</Text>
        </Button>
      </Row>
      {config().skip.length === 0 ? null : (
        config().skip.map((exe) => (
          <View class="flex-row items-center gap-3">
            <Text class="flex-1 text-xs text-muted-foreground">{exe}</Text>
            <Button class={BTN_GHOST} onPress={() => removeSkip(exe)}>
              <Text class={LABEL}>移出</Text>
            </Button>
          </View>
        ))
      )}
    </Card>
  );
}

function ModeCard(): JSX.Element {
  /** 写成函数而不是常量：组件体只跑一次，常量会冻在首帧。 */
  const isHook = () => config().mode === "hook";
  return (
    <Card title="实现方式与透传" hint="钩子能按速度智能变速；兼容模式只是改系统的固定值">
      <Row
        label="变速方式"
        hint={isHook() ? "按曲线改每一格；提权窗口里不生效" : "直接改 Windows 的「每格滚几行」，不装钩子"}
      >
        <View class="flex-row gap-1 rounded-full border border-border p-1">
          <Button class={isHook() ? SEG_ON : SEG_OFF} onPress={() => applyConfig({ ...config(), mode: "hook" })}>
            <Text class={isHook() ? LABEL_PRIMARY : LABEL}>钩子</Text>
          </Button>
          <Button class={isHook() ? SEG_OFF : SEG_ON} onPress={() => applyConfig({ ...config(), mode: "system" })}>
            <Text class={isHook() ? LABEL : LABEL_PRIMARY}>兼容</Text>
          </Button>
        </View>
      </Row>
      {isHook() ? null : (
        <Row label="每格行数" hint={`系统当前是 ${meta.systemLines} 行；退出兼容模式时自动还原`}>
          <View class="flex-row items-center gap-3">
            <Value text={`${config().systemLines} 行`} />
            <Slider
              class={SLIDER}
              value={config().systemLines}
              min={1}
              max={25}
              step={1}
              onChange={(event) =>
                applyConfig({ ...config(), systemLines: Math.round(single(event.value)) })
              }
              trackClass={SLIDER_TRACK}
              indicatorClass={SLIDER_INDICATOR}
              rangeClass={SLIDER_RANGE}
              thumbClass={SLIDER_THUMB}
            />
          </View>
        </Row>
      )}
      <Row label="平滑滚动" hint="一次大跳拆成小格按节拍发（无级感）；关掉更跟手，但会一跳一跳">
        <Switch
          checked={config().smooth}
          onChange={(event) => applyConfig({ ...config(), smooth: event.checked })}
          trackClass={SWITCH_TRACK}
          thumbClass={SWITCH_THUMB}
        />
      </Row>
      <Row label="Ctrl 按住时不改" hint="保留「Ctrl+滚轮」缩放">
        <Switch
          checked={config().passCtrl}
          onChange={(event) => applyConfig({ ...config(), passCtrl: event.checked })}
          trackClass={SWITCH_TRACK}
          thumbClass={SWITCH_THUMB}
        />
      </Row>
      <Row label="Alt 按住时不改">
        <Switch
          checked={config().passAlt}
          onChange={(event) => applyConfig({ ...config(), passAlt: event.checked })}
          trackClass={SWITCH_TRACK}
          thumbClass={SWITCH_THUMB}
        />
      </Row>
      <Row label="Shift 按住时不改" hint="有些程序用 Shift+滚轮横向滚">
        <Switch
          checked={config().passShift}
          onChange={(event) => applyConfig({ ...config(), passShift: event.checked })}
          trackClass={SWITCH_TRACK}
          thumbClass={SWITCH_THUMB}
        />
      </Row>
    </Card>
  );
}

/** wheel 单位 → 格，保留一位小数（细粒度滚轮一格会被拆成好几个事件）。 */
function notches(units: number): string {
  return `${(units / 120).toFixed(1)} 格`;
}

/** 累计加速比：发出去的格数 ÷ 进来的格数。 */
function ratioText(): string {
  const state = snap()?.stats;
  if (!state || state.inUnits === 0) return "—";
  return `${(state.outUnits / state.inUnits).toFixed(2)}x`;
}

/**
 * 统计条：钉在标题栏下面，不随内容滚走；收起时只有一行累计量。
 *
 * 用 `nsh/base` 的 `Collapsible`，`open` 由这边持有，触发器是一个 `role="button"` 的 View。
 */
function StatsStrip(): JSX.Element {
  /** 写成函数：组件体只跑一次，常量会冻在首帧。 */
  const failed = () => snap()?.stats.injectFailed ?? 0;
  return (
    <Collapsible
      class="border-b border-border bg-card"
      open={statsOpen()}
      reveal
      content={
        <View class="flex-col gap-3 px-4 pb-3">
          <View class="flex-row gap-3">
            <Metric label="进来" value={notches(snap()?.stats.inUnits ?? 0)} />
            <Metric label="发给应用" value={notches(snap()?.stats.outUnits ?? 0)} />
            <Metric label="加速比" value={ratioText()} />
            <Metric label="事件" value={`${snap()?.stats.events ?? 0}`} />
          </View>
          <View class="flex-row gap-3">
            <Metric label="原样放行" value={`${snap()?.stats.passed ?? 0}`} />
            <Metric label="注入失败" value={`${failed()}`} tone={failed() > 0 ? "danger" : "normal"} />
            <Metric label="当前速度" value={speedText(snap()?.stats.velocity ?? 0)} />
            <Metric label="当前倍率" value={gainText(snap()?.stats.gain ?? 1)} />
          </View>
          <Caption>
            {`最近一格原始值：${snap()?.stats.lastDelta ?? 0}（标准一格是 ±120；不是 ±120 说明鼠标在送细粒度值）`}
          </Caption>
          {tested() ? (
            <View class="flex-col gap-1">
              <Caption>自检明细（每 40ms 一格，连滚 12 格）</Caption>
              <View class="flex-row flex-wrap gap-2">
                {tested()!.steps.map((step, index) => (
                  <Text class="text-xs text-muted-foreground">
                    {`#${index + 1} ${step.velocity.toFixed(0)}/s ${gainText(step.gain)} → ${step.out}`}
                  </Text>
                ))}
              </View>
            </View>
          ) : null}
        </View>
      }
    >
      <View
        class="flex-row items-center gap-4 px-4 py-1"
        role="button"
        onPress={() => statsOpen.set(!statsOpen())}
      >
        <Text class="text-xs font-medium text-foreground">统计</Text>
        <Text class="text-xs text-muted-foreground">{`进来 ${notches(snap()?.stats.inUnits ?? 0)}`}</Text>
        <Text class="text-xs text-muted-foreground">{`发出 ${notches(snap()?.stats.outUnits ?? 0)}`}</Text>
        <Text class="text-xs font-semibold text-foreground">{ratioText()}</Text>
        <Text class={failed() > 0 ? "text-xs font-semibold text-destructive" : "text-xs text-muted-foreground"}>
          {`注入失败 ${failed()}`}
        </Text>
        <View class="flex-1" />
        <Text class="text-xs text-muted-foreground">{statsOpen() ? "收起" : "展开"}</Text>
      </View>
    </Collapsible>
  );
}

function LogCard(): JSX.Element {
  return (
    <Card
      title="日志"
      hint={`落盘：${meta.logDir}`}
      right={
        <Button class={BTN_GHOST} onPress={() => api.clearLogs()}>
          <Text class={LABEL}>清空</Text>
        </Button>
      }
    >
      <View class="flex-col gap-1 rounded-(--radius) bg-muted p-3">
        {(snap()?.logs ?? []).slice(-30).map((line) => (
          <Text class="text-xs text-muted-foreground">{line}</Text>
        ))}
      </View>
    </Card>
  );
}

function Footer(): JSX.Element {
  return (
    <View class="flex-col gap-2 pb-2">
      {meta.hint ? (
        <View class="flex-row items-center gap-3">
          <Note ok={false} text={meta.hint} />
          <Button class={BTN_GHOST} onPress={relaunchElevated}>
            <Text class={LABEL}>以管理员重启</Text>
          </Button>
        </View>
      ) : (
        <Caption>已具备管理员权限：提权窗口里的滚轮也会变速。</Caption>
      )}
      <Caption>{`版本 ${meta.version} · 配置 ${meta.configPath}`}</Caption>
      <Caption>关窗会收进托盘，变速继续生效；真退出请用托盘菜单。</Caption>
    </View>
  );
}

export function Settings(): JSX.Element {
  bindWindow();
  interceptClose();
  installTray({ enabled: () => config().enabled, onToggle: toggleEnabled });

  return (
    <View class="flex-1 flex-col bg-background">
      <Header />
      <StatsStrip />
      {message() ? (
        <View class="bg-card px-4 py-2">
          <Note ok={message()!.ok} text={message()!.text} />
        </View>
      ) : null}
      {/* contentOffset 钉死从顶部开始：内容比窗口高时，默认落点不是顶部。 */}
      <Scroll class="flex-1" contentOffset={{ x: 0, y: 0 }}>
        <View class="flex-col gap-3 px-4 pb-4 pt-1">
          <CurveCard />
          <ModeCard />
          <AppsCard />
          <LogCard />
          <Footer />
        </View>
      </Scroll>
    </View>
  );
}






