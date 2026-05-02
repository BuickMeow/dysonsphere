# 🏔️ 屎山指数报告 v4.0 —— 爆音残余 + Release 过短根因诊断

**项目名称**: Dysonphere（戴森球）合成器引擎  
**分析日期**: 2026-05-02（最新代码状态）  
**对比基准**: xsynth  
**用户反馈**: 爆音有进步但还有一点 → Release有进步但还是有点短  

---

## 📊 v3→v4 修改确认

已应用的修复（从代码中确认）：
- ✅ `soft_clip` 重写：`x.signum() * (1.0 - 0.5/ax)`，保证 `|output| ≤ 1.0`
- ✅ `MASTER_GAIN = 0.15` 已添加
- ✅ `steal_voice` 改为移除非release voice中最安静的
- ✅ `sf2.rs` release floor `< 0.05s → 0.5s` 兜底已应用

---

## 🔍 残余爆音分析：三重来源

### 来源 1（🔴 大概率根因）：LoopSustain/LoopContinuous 在 `loop_start == loop_end` 时 Position 无限增长

**问题代码** (`sampler.rs:88-96`)：

```rust
if self.position >= self.loop_end as f64 {
    match self.loop_mode {
        LoopMode::LoopContinuous => {
            self.position -= (self.loop_end - self.loop_start) as f64;
        }
        LoopMode::LoopSustain if !self.released => {
            self.position_at_release = self.position;
            self.position -= (self.loop_end - self.loop_start) as f64;
        }
    }
}
```

**根因**: 当 `loop_end == loop_start` 时，`position -= 0`，position 从不 wrap，持续增长。当 position 跨越 `sample_end` 后，输出从正常值突变为 0.0，产生 click。

**触发条件**: SF2 文件中 `SampleModes = 1` (LoopContinuous) 但 `loop_start == loop_end == 0`（某些乐器区域没有显式的 loop 范围但设置了 loop mode）。

**xsynth 的保护** (`soundfont/mod.rs:334-340`)：

```rust
let loop_params = LoopParams {
    mode: if region.loop_start == region.loop_end {
        LoopMode::NoLoop  // <-- 关键保护
    } else {
        region.loop_mode
    },
    // ...
};
```

**dysonphere 的缺失** (`sf2.rs:471-473`)：

```rust
let loop_mode = pzone
    .loop_mode
    .unwrap_or(izone.loop_mode.unwrap_or(LoopMode::NoLoop));
// ← 没有检查 loop_start == loop_end！
```

**修复**: 在 `sf2.rs` 的 `build_presets` 中，loop_mode 赋值之前添加保护：

```rust
// 在 loop_start 和 loop_end 计算之后：
let loop_mode = if loop_start == loop_end {
    LoopMode::NoLoop
} else {
    pzone.loop_mode.unwrap_or(izone.loop_mode.unwrap_or(LoopMode::NoLoop))
};
```

同样需要在 `sfz.rs` 中添加相同的保护。

---

### 来源 2（🟡 次要原因）：Envelope Attack 后进入 Hold/Decay 的跳变

**问题代码** (`envelope.rs:80-83`)：

```rust
// Decay
StageParams {
    target: descriptor.sustain,
    duration_samples: (descriptor.decay * sr).round() as u32,
},
```

**根因**: Decay 阶段没有 `.max(0.001)` 保护。如果 SF2/SFZ 的 decay = 0.0（未设置），`duration_samples = 0`，value 直接从 Hold 的 1.0 跳到 Sustain level。如果 Sustain 是较低值（如 0.4），这个瞬间跳变产生高频能量。

对比 Attack 阶段有保护：
```rust
duration_samples: (descriptor.attack.max(0.001) * sr).round() as u32,  // ← 有 .max(0.001) 保护
```

**修复**: 为 Hold 和 Decay 添加最小 duration 保护：
```rust
// Decay
StageParams {
    target: descriptor.sustain,
    duration_samples: (descriptor.decay.max(0.001) * sr).round() as u32,
},
// Hold
StageParams {
    target: 1.0,
    duration_samples: (descriptor.hold.max(0.001) * sr).round() as u32,
},
```

虽然 Hold 的 target 和 start 相同（都是 1.0），duration=0 不会造成跳变。但 Decay 从 1.0 到 sustain（可能低于 1.0）需要保护。

**注意**：如果 sustain = 1.0（默认），decay=0 不会产生任何跳变（因为值不变）。只有在 sustain < 1.0 且 decay=0 时才会产生爆音。这解释了为什么用户在某些音色上听到爆音，另一些则没有——取决于 SF2 文件中的 sustain 设置。

---

### 来源 3（🟢 低频事件）：Voice Steal 移除有输出值的 Voice

**当前代码** (`synth.rs:501-517`)：

```rust
fn steal_voice(&mut self) {
    if let Some(idx) = self.voices.iter().position(|(_, v)| v.is_releasing()) {
        self.voices.swap_remove(idx);
        return;
    }
    if let Some((quietest_idx, _)) = self.voices.iter().enumerate()
        .min_by_key(|(_, (_, v))| v.velocity) {
        self.voices.swap_remove(quietest_idx);
    }
}
```

**根因**: `steal_voice` 在 event flush 期间调用（不在渲染中间），所以被移除的 voice 不会在渲染循环中产生 click。但如果某个 voice 在 sustain 阶段且 velocity 很低，它可能被移除，而它的输出值不是 0（sustain level）—— 这本身不是问题，因为 voice 被移除后不再参与渲染。**但这个 voice 的贡献从混音中消失**，在 voice 被移除后的下一帧渲染中，mix 值会突变。

但这只发生在 voice count > 256 时（MAX_VOICES）。对于大多数正常使用场景，256 个 voice 足够。

**修复**: 在 steal 时给被移除的 voice 一个短 kill release，通过 `voice.kill()` 让它在下一次渲染中自然 fade out，而不是直接移除。但这需要改变 steal 不再直接 swap_remove，而是在渲染循环中检查。

---

## 🔍 Release 过短分析：三重根因

### 根因 1（🔴 主要）：Velocity 对 Release 的无条件缩放

**问题代码** (`envelope.rs:61-62`)：

```rust
let vel_norm = vel as f32 / 127.0;
let release = (descriptor.release * (0.2 + vel_norm * 0.8)).max(0.05);
```

**数值验证**：

| 音符 velocity | vel_norm | 缩放因子 | descriptor.release=0.5s 的实际值 |
|-------------|----------|---------|-------------------------------|
| 127 (ff)    | 1.000    | 1.000   | 0.500s ✅ |
| 100 (mf)    | 0.787    | 0.830   | 0.415s ⚠️ |
| **64 (mp)** | **0.504** | **0.603** | **0.302s** 🔴 |
| 32 (pp)    | 0.252    | 0.402   | 0.201s 🔴 |
| 1 (ppp)    | 0.008    | 0.206   | 0.103s 🔴 |

对于一个 `vel=64` 的钢琴音符（中等力度），descriptor 中 0.5s 的 release 被缩放到只有 **0.3 秒**。加上 SF2 解析的 `max(0.05)` floor（如果 descriptor 本身传入的 release < 0.05s 才会提升到 0.05s），实际 floor 只防住了极端情况。

**xsynth 的做法**: 不对 release 做 velocity 自动缩放。Velocity 只通过 modulator 影响 volume 和 filter，不影响 envelope 时间。

**修复**: 移除或大幅减小 velocity 对 release 的影响。推荐两个方案：

**方案 A（保守）**: 将缩放范围从 [0.2, 1.0] 改为 [0.5, 1.0]：
```rust
let release = descriptor.release * (0.5 + vel_norm * 0.5);
```
vel=64 时：0.5 * (0.5 + 0.504*0.5) = 0.5 * 0.752 = 0.376s。仍然偏短但可接受。

**方案 B（推荐，对齐 xsynth）**: 完全移除 velocity 缩放：
```rust
let release = descriptor.release.max(0.05);
```

---

### 根因 2（🟡）：Release Floor 太低

**问题代码** (`envelope.rs:62`)：

```rust
let release = (descriptor.release * ...).max(0.05);
```

**根因**: floor 0.05s = 50ms。对于大多数音乐场景，50ms 的 release 几乎感知为 "click"。钢琴的 natural release 通常需要 0.5-2.0 秒。

**xsynth 的 practice**: 不强制 floor，而是依赖 SF2 文件的 envelope 值和 modulators。

**修复**: 将 floor 提升到 0.2s：
```rust
let release = (descriptor.release * ...).max(0.2);
```

---

### 根因 3（🟢）：SF2 显式 Release 值小于兜底阈值但大于 floor

**当前代码** (`sf2.rs:438-445`)：

```rust
release: {
    let secs = timecents_to_seconds(
        timecents_merge(-12000, pzone.env_release, izone.env_release) as f32,
    );
    if secs < 0.05 { 0.5 } else { secs }
},
```

**根因**: 如果 SF2 文件在 instrument zone 中显式设置了 `ReleaseVolEnv` generator（即使值很小，如 0.06s），`secs >= 0.05`，不触发 0.5s 兜底。然后 `envelope.rs` 中 `.max(0.05)` 确保至少 0.05s。再经过 velocity 缩放（如 vel=64 时 *0.6），最终只有 0.06*0.6 = 0.036s，取 `max(0.05)` 后为 0.05s。**这就是为什么 release 听起来仍然短**。

**修复**: 提升 sf2.rs 中的无条件 floor（与 envelope.rs 的 floor 配合）：
```rust
release: {
    let secs = timecents_to_seconds(
        timecents_merge(-12000, pzone.env_release, izone.env_release) as f32,
    );
    if secs < 0.05 { 0.8 } else { secs.max(0.2) }
},
```

---

## 🛠️ 修复方案（完整代码修改）

### 修改 1：`dysonphere-soundfont/src/sf2.rs` — Loop 保护

在 `build_presets` 中（第 471-473 行附近），**在 loop_start 和 loop_end 计算之后**：

```rust
// 在 loop_start 和 loop_end 计算（第 396-409 行）之后，在 loop_mode 赋值之前添加
let loop_mode = {
    let raw = pzone.loop_mode.unwrap_or(izone.loop_mode.unwrap_or(LoopMode::NoLoop));
    if loop_start == loop_end && raw != LoopMode::NoLoop {
        LoopMode::NoLoop  // 保护：无有效 loop 范围时禁用 looping
    } else {
        raw
    }
};
```

### 修改 2：`dysonphere-core/src/envelope.rs` — Decay 保护 + Velocity 重校准 + Floor 提升

```rust
// 第 61-62 行，替换
// let vel_norm = vel as f32 / 127.0;
// let release = (descriptor.release * (0.2 + vel_norm * 0.8)).max(0.05);

// 改为：
let vel_norm = vel as f32 / 127.0;
let release = (descriptor.release * (0.5 + vel_norm * 0.5)).max(0.2);
```

```rust
// 第 78-80 行，Decay 添加 max 保护
// 当前：
// StageParams {
//     target: descriptor.sustain,
//     duration_samples: (descriptor.decay * sr).round() as u32,
// },

// 改为：
StageParams {
    target: descriptor.sustain,
    duration_samples: (descriptor.decay.max(0.001) * sr).round() as u32,
},
```

### 修改 3：`dysonphere-soundfont/src/sf2.rs` — Release floor 提升

```rust
// 第 438-445 行，替换
release: {
    let secs = timecents_to_seconds(
        timecents_merge(-12000, pzone.env_release, izone.env_release) as f32,
    );
    if secs < 0.05 { 0.5 } else { secs }
},

// 改为：
release: {
    let secs = timecents_to_seconds(
        timecents_merge(-12000, pzone.env_release, izone.env_release) as f32,
    );
    // 低于阈值则使用兜底，否则至少 0.3s
    if secs < 0.05 { 0.8 } else { secs.max(0.3) }
},
```

### 修改 4：`dysonphere-soundfont/src/sfz.rs` — Loop 保护（相同逻辑）

在 `load` 函数中，`Region` 构建处（region 的 loop_mode 赋值）：

```rust
// 在 volume 计算附近（约第 71-89 行）
let loop_mode = if loop_start == loop_end && region.loop_mode != LoopMode::NoLoop {
    LoopMode::NoLoop
} else {
    region.loop_mode
};
```

---

## 📋 v4 优先级总结

### 🔴 立刻改（预计 30 分钟）

| # | 文件 | 修改 | 预期效果 |
|---|------|------|---------|
| 1 | `sf2.rs` | loop_start==loop_end → NoLoop 保护 | 消除 position 无限增长导致的 click |
| 2 | `envelope.rs` | velocity 缩放范围改为 [0.5, 1.0]，floor 改为 0.2s | Release 主观长度接近设定值 |
| 3 | `envelope.rs` | Decay 添加 `.max(0.001)` 保护 | 消除 sustain<1.0 时的跳变 |

### 🟡 随后改（预计 15 分钟）

| # | 文件 | 修改 | 预期效果 |
|---|------|------|---------|
| 4 | `sf2.rs` | release floor 提升到 0.8s / 0.3s | Release 更自然 |
| 5 | `sfz.rs` | loop_start==loop_end → NoLoop 保护 | 对齐 sf2.rs 的保护 |

---

## 🧠 复盘：Release 过短的三层漏斗

用户听到的 release 时间经过了**三层缩减**：

```
SF2 原始 release → sf2.rs floor 检测 → envelope.rs velocity 缩放 → envelope.rs floor

例：vel=64, instrument 显式 release=0.06s
  SF2 解析后:  0.06s
  sf2 floor:   ≥0.05 → 不触发兜底 → 0.06s
  vel 缩放:    0.06 * (0.2+0.5*0.8) = 0.06 * 0.6 = 0.036s
  env floor:   0.036 < 0.05 → max(0.05) → 0.05s
  最终:         0.05 秒！ ← 几乎瞬死
```

而 xsynth 的同一声音：
```
  SF2 解析后:  0.06s → 直接使用 → 0.06s
  最终:         0.06 秒（虽然也短，但没有经过缩放和 floor 的额外缩减）
```

再加上 xsynth 的 CC72 (release time) 和 modulators 可能增加 release，而 dysonphere 没有这些。

---

*本报告基于对 v3 修改后的最新代码分析生成。上述 5 项修改可在一个小时内完成，预期能消除残余爆音并将 release 感知时间提升到接近 xsynth 的水平。*
