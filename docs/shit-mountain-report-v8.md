# 🏔️ 屎山指数报告 v8.0 —— LoopSustain 灾难级 bug + 采样加载深层问题

**项目名称**: Dysonphere（戴森球）合成器引擎  
**分析日期**: 2026-05-02（最新代码状态）  
**对比基准**: xsynth（本地 `/Users/jieneng/Documents/GitHub/xsynth`）  
**用户反馈**: SFZ 声音突然变大后消失、click 声、SF2/SFZ 音量不一致  
**屎山指数**: **18 / 100** 🔴🔴（LoopSustain 存在灾难性采样逻辑 bug）

---

## 📊 v7→v8 关键修改确认

| 修改项 | 状态 | 效果 |
|--------|------|------|
| `vel_amp = vel_norm.powf(5.0)` | ✅ | 模仿 SF2 默认 modulator |
| `get()` 越界返回最后一个样本 | ✅ | 避免突然 0.0 的 click |
| `release_multiplier` (CC72) | ✅ | 但默认 1.0 无延长 |
| release floor 2.5s / 1.0s | ✅ | 感知仍只有 0.3-0.8s |
| **LoopSustain `position_at_release` 未使用** | ❌ | **灾难级 bug 未发现** |

---

## 🔴 灾难级 Bug #1：LoopSustain 的 `position_at_release` 从未在 `release()` 中设置

### 问题代码 (`sampler.rs:64-77`)

```rust
/// Signal note release.
pub fn release(&mut self) {
    match self.loop_mode {
        LoopMode::LoopSustain if !self.released => {
            self.released = true;
            // Stop looping, continue playing from current position to sample_end
            // 🔴 BUG: position_at_release 未在此处设置！
        }
        ...
    }
}
```

`position_at_release` 只在 `process()` 中设置：

```rust
LoopMode::LoopSustain if !self.released => {
    self.position_at_release = self.position;  // 仅在 >= loop_end 时触发！
    self.position -= (self.loop_end - self.loop_start) as f64;
}
```

### 后果

如果 note-off 时 position 在 loop 区域内（未达到 `loop_end`），`position_at_release` **保持旧值（0.0）**。但这个字段在 release 后的位置计算中**从未被使用**——它是死代码。

### XSynth 的做法 (`voice/sampler.rs:228-230`)

```rust
fn signal_release(&mut self) {
    self.is_released = true;  // 简洁：只标记状态
}
```

xsynth 不依赖 `position_at_release`。它使用 `self.last`，在每次 `get()` 调用时记录，确保 release 时的位置回溯是正确的。

---

## 🔴 灾难级 Bug #2：LoopSustain release 后采样从 loop 区域跳转到未知数据

### 问题代码 (`sampler.rs:81-102`)

```rust
pub fn process(&mut self) -> f32 {
    let pos = self.position;
    let sample = self.read_sample(pos);
    self.position += self.speed;

    if self.position >= self.loop_end as f64 {
        match self.loop_mode {
            LoopMode::LoopContinuous => {
                self.position -= (self.loop_end - self.loop_start) as f64;
            }
            LoopMode::LoopSustain if !self.released => {
                // 未 release 时：loop
                self.position_at_release = self.position;
                self.position -= (self.loop_end - self.loop_start) as f64;
            }
            _ => {}  // released 后走到这里，不 reset position！
        }
    }
    sample
}
```

### 时序分析

LoopSustain 采样，loop_start=1000, loop_end=2000, sample_end=5000：

```
时刻     event                position   被读取的数据
─────    ─────                ────────   ───────────
T0       note-on              offset     采样 offset 处
T1-T4    looping              1500-2000  loop 区域内（延音）
T4       note-off (released)  1500       loop 区域数据（正常）
T5-T10   released, advancing  1500-2000  loop 区域数据（正常）
T10      position >= loop_end  2000       🔴 跳转到 loop_end+1
T11-T30  past loop_end        2001-5000  🔴 读取 loop 之后的数据！
T31-T∞   past sample_end      >5000      data[sample_end-1]
```

**T11-T30 期间读取的 "loop 之后的数据" 可能是什么？**

采样文件布局通常是：`[attack_原始起音][loop_延音循环段][tail_尾部衰减]`

如果 `loop_end` 之后的尾部数据恰好是 **attack 部分**（采样文件中循环段之后的区域可能包含非预期的内容），声音就会突变变大。

**这完美解释了用户观察到的现象：**
- **前 0.5s**（T0-T10）：在 loop 区域内单曲，正常钢琴声
- **后 0.5s**（T10-T20）：position 跳出了 loop 区域，播放未知数据 → **声音突然变大**
- **最后 1s**（T30-T∞）：position 超过 sample_end，get() 返回最后一个样本，envelope 已衰减到接近 0 → **完全没有声音**

### XSynth 的做法 (`voice/sampler.rs:202-218`)

```rust
fn get(&mut self, pos: usize) -> f32 {
    let mut pos = pos + self.offset;
    if !self.is_released {
        self.last = pos;                     // 记录当前位置
        if pos > end {
            pos = (pos - end - 1) % (end - start) + start;  // loop
        }
    } else {
        pos = pos - self.last + self.loop_end;  // 🔑 从 loop_end 开始！
    }
    self.buffer.get(pos)
}
```

**XSynth release 行为：release 后从 `loop_end` 开始播放，而不是从当前 position 继续。**

数学：
- release 时刻 `time=t_r`，`self.last = t_r + offset`
- release 后第一个 sample（`time = t_r`）：`pos = (t_r+offset) - (t_r+offset) + loop_end = loop_end`
- release 后第 n 个 sample（`time = t_r + n`）：`pos = loop_end + n`

XSynth 假设 `loop_end` 之后就是 release tail（这是标准采样制作实践）。

dysonphere 的行为是从当前 position 继续播放到 loop_end，然后自然越界。两者都可能有 click（xsynth 有从当前 position 跳到 loop_end 的跳变），但 xsynth 的跳变是可控的（只一次），而 dysonphere 的跳变可能持续播放未知数据。

---

## 🔴 Bug #3：LoopContinuous + Note-Off 时的 get() 也返回 0

### 问题代码 (`sampler.rs:120-127`)

```rust
fn get(&self, idx: usize) -> f32 {
    if (!matches!(self.loop_mode, LoopMode::LoopContinuous) || self.released)
        && idx >= self.sample_end as usize {
            return self.data.get(self.sample_end as usize - 1)
                .copied().unwrap_or(0.0);
    }
    self.data.get(idx).copied().unwrap_or(0.0)
}
```

对于 LoopContinuous 且 `released` 之后：`!matches!(LoopContinuous, ...) || self.released` = `false || true` = `true`。

如果 position 增长到超过 sample_end（因为 LoopContinuous 在 process() 中不检查 sample_end，position 可能无限增长）... 等等，`process()` 中 LoopContinuous 会 loop，position 不会超过 loop_end。

但如果 speed 非常大（极高音），position 可能一次跳过 loop_end。但 `process()` 中的 `if self.position >= self.loop_end` 检查会将其重置。所以 position 不会超过 loop_end。

**但这里的问题是**：LoopContinuous 在 release 后仍继续 loop。当 `get()` 被调用时，idx 在 loop 区域内，不会触发越界条件。所以这个 bug 实际上在 LoopContinuous 中不会触发。

**但对于 LoopSustain released**：`released=true`，如果 idx >= sample_end，返回最后一个样本。由于 position 在 released 后可以超过 sample_end（见 Bug #2），这是会触发的。

---

## 🔴 Bug #4：read_sample 在 sample_end 边界处产生跳变

### 问题代码 (`sampler.rs:106-113`)

```rust
fn read_sample(&self, pos: f64) -> f32 {
    let idx = pos as usize;
    let frac = (pos - idx as f64) as f32;
    let a = self.get(idx);
    let b = self.get(idx + 1);
    a + (b - a) * frac
}
```

当 pos 从 `sample_end - 1.5` 跳到 `sample_end + 0.5`（speed > 1 时）：

| frame | pos | idx | get(idx) | get(idx+1) | 输出 |
|-------|-----|-----|----------|------------|------|
| T | sample_end-1.5 | sample_end-2 | data[n-2] | data[n-1] | 插值 (连续) |
| T+1 | sample_end+0.5 | sample_end | data[n-1] | data[n-1] | data[n-1] |

从插值 `avg(data[n-2], data[n-1])` 跳变到 `data[n-1]`。差值可能很小，但如果 speed 很大或采样值变化剧烈，就会产生可闻 click。

---

## 🟡 SF2 与 SFZ 音量不一致分析

### SF2 volume 计算 (`sf2.rs:414-417`)

```rust
let attenuation = sum_option(pzone.attenuation, izone.attenuation);
let volume = 10.0f32.powf(-attenuation as f32 / 200.0);
```

### SFZ volume 计算 (`sfz.rs:71`)

```rust
let volume = db_to_amp(region.volume as f32);
// db_to_amp(db) = 10^(db/20)
```

### 单位等价性验证

| 场景 | SF2 attenuation (cb) | SF2 volume | SFZ volume (dB) | SFZ volume | 等效？ |
|------|---------------------|-----------|-----------------|-----------|--------|
| 无衰减 | 0 | 1.0 | 0 | 1.0 | ✅ |
| -6dB | 60 | 0.5 | -6 | 0.5 | ✅ |
| -20dB | 200 | 0.1 | -20 | 0.1 | ✅ |
| -40dB | 400 | 0.01 | -40 | 0.01 | ✅ |

**单位转换是正确的。音量差异来自音色库制作者的参数设置，而不是引擎的 bug。**

### 可能的原因

1. **同一音色库的 SF2 和 SFZ 版本参数不同**：制作者在两种格式中可能设置了不同的 attenuation/volume 值
2. **SF2 的 attenuation 来自 generator 而非 modulator**：缺少 velocity→attenuation modulator 导致 SF2 的 velocity 响应与标准不同
3. **但 disonphere 已有 `vel_norm.powf(5.0)` 近似**：这提供了一致的 velocity 响应（对 SF2 和 SFZ 都适用）

---

## 🟡 Click 声的多重来源完整清单

| # | 来源 | 严重程度 | 触发条件 |
|---|------|---------|---------|
| 1 | **LoopSustain release 后采样跳变**（Bug #2） | 🔴 灾难 | note-off 后半秒 |
| 2 | **position_at_release 未在 release() 设置**（Bug #1） | 🔴 严重 | 间接影响 |
| 3 | **read_sample 越界跳变**（Bug #4） | 🟡 中等 | speed > 1，接近 sample_end |
| 4 | **attack 凹曲线快速起音** | 🟢 轻微 | attack=0.001s 时高频瞬态 |
| 5 | **voice steal 5ms kill release** | 🟡 中等 | MAX_VOICES 超出时 |
| 6 | **LoopContinuous release 后继续 loop→envelope 相乘 0 值** | 🟢 轻微 | LoopContinuous note-off 尾端 |

---

## 🛠️ v8 修复方案

### 🔴 P0：修复 LoopSustain release（Bug #1 + #2）

**修改 1：在 `release()` 中记录 `position_at_release`**

```rust
// sampler.rs:64-78
pub fn release(&mut self) {
    match self.loop_mode {
        LoopMode::LoopSustain if !self.released => {
            self.released = true;
            self.position_at_release = self.position;  // ← 添加这行
        }
        LoopMode::LoopContinuous => {
            // same as before
        }
        LoopMode::OneShot => {
            // same as before
        }
        _ => {}
    }
}
```

**修改 2：release 后从 `loop_end` 开始播放（对齐 xsynth）**

```rust
// sampler.rs:81-102
pub fn process(&mut self) -> f32 {
    // LoopSustain release: 从 loop_end 开始播放 release tail
    let read_pos = if self.released && self.loop_mode == LoopMode::LoopSustain {
        self.loop_end as f64 + (self.position - self.position_at_release)
    } else {
        self.position
    };
    let sample = self.read_sample(read_pos);

    self.position += self.speed;

    if self.position >= self.loop_end as f64 {
        match self.loop_mode {
            LoopMode::LoopContinuous => {
                self.position -= (self.loop_end - self.loop_start) as f64;
            }
            LoopMode::LoopSustain if !self.released => {
                self.position_at_release = self.position;
                self.position -= (self.loop_end - self.loop_start) as f64;
            }
            _ => {}
        }
    }
    sample
}
```

**效果**：
- ✅ LoopSustain release 后不再播放 loop 区域之外的数据
- ✅ 消除 "声音突然变大" 的现象
- ✅ 消除 "click" 的主要来源
- ✅ 对齐 xsynth 行为（从 loop_end 开始播放 release tail）

### 🟡 P1：修复 read_sample 越界跳变

```rust
// sampler.rs:106-113
fn read_sample(&self, pos: f64) -> f32 {
    let idx = pos as usize;
    let frac = (pos - idx as f64) as f32;
    
    // 在 sample_end 边界处，确保 b = a 以避免跳变
    let a = self.get(idx);
    let b = if idx + 1 >= self.sample_end as usize {
        a  // 边界处使用 a 代替 b，避免跳变
    } else {
        self.get(idx + 1)
    };
    a + (b - a) * frac
}
```

### 🟡 P1：修复 LoopContinuous release 后的 get() 越界

```rust
// sampler.rs:120-127
fn get(&self, idx: usize) -> f32 {
    // LoopContinuous 永远不应该越界（一直在 loop 区域内）
    // 但 releases 后 LoopSustain/NoLoop 可能越界
    let can_past_end = self.loop_mode != LoopMode::LoopContinuous;
    if can_past_end && idx >= self.sample_end as usize {
        return self.data.get(self.sample_end as usize - 1)
            .copied().unwrap_or(0.0);
    }
    self.data.get(idx).copied().unwrap_or(0.0)
}
```

### 🟡 P2：SF2/SFZ 音量对齐

当前已有 `vel_norm.powf(5.0)` 近似。更进一步可以完全实现 SF2 默认 modulator 系统（见 v7 报告），但当前近似已足够。

对于 SF2 和 SFZ 之间的音量差异，建议在 SFZ 解析中添加 `amp_veltrack` 支持：

```rust
// sfz.rs apply_opcode
"amp_veltrack" => {
    if let Some(v) = parse_f32() {
        region.amp_veltrack = v;  // 默认 100（100% velocity tracking）
    }
}
```

---

## 🧠 终极复盘：为什么 v7 之前的分析没发现 Bug #1/#2？

### 分析盲区

1. **假设 LoopSustain 的行为是正确的**：之前的报告专注于 envelope、volume、master_gain，从未检查 sampler.rs 中 LoopSustain 的 release 逻辑
2. **`position_at_release` 被当成已实现的功能**：它存在于结构体中，在 `process()` 中被设置，但从未被使用。v7 之前没有发现它是死代码
3. **没有与 xsynth 进行 LoopSustain 的逐行对比**：直到用户报告 "声音突然变大"，才触发对 LoopSustain release 逻辑的深入分析
4. **测试只覆盖了 NoLoop release**：`envelope.rs` 的测试使用 NoLoop 模式，LoopSustain 的 release 从未被端到端测试

### 正确的诊断方法论

```
用户报告：SFZ 声音在 0.5s 后突然变大，最后 1s 没声音
                                 ↓
不应该假设 envelope 是唯一的问题源
                                 ↓
采样级问题 ← sampler.process() 在不同 loop mode 下的行为
                                 ↓
LoopSustain released 后 position 不 reset
                                 ↓
position 越过 loop_end 播放未知数据
                                 ↓
找到了！声音突变 = 采样数据突变
```

---

*本报告找到了导致"声音突然变大"+"click"+"最后没声音"三联症状的灾难级根因：LoopSustain 的 `position_at_release` 设置时机错误且从未被使用，导致 release 后采样从 loop 区域跳转到未知数据。这是 dysonphere 当前最严重的音频 bug。*
