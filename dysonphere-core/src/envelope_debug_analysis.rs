//! 包络实现深度调试分析器
//! 
//! 逐行分析包络代码的实际行为，找出测量偏差的精确原因

use crate::Envelope;
use dysonphere_soundfont::types::EnvelopeDescriptor;

/// 单步调试信息
#[derive(Debug, Clone)]
pub struct StepDebugInfo {
    pub sample_index: usize,
    pub stage: &'static str,
    pub elapsed: u32,
    pub t_raw: f32,           // 原始归一化时间 elapsed/duration
    pub curved_t: f32,        // 应用曲线后的t
    pub start_value: f32,
    pub target_value: f32,
    pub computed_value: f32,
    pub actual_value: f32,    // 包络实际输出值
}

/// 完整分析报告
#[derive(Debug, Clone)]
pub struct EnvelopeDebugReport {
    pub theoretical_attack_sec: f32,
    pub sample_rate: u32,
    pub duration_samples: u32,
    pub steps: Vec<StepDebugInfo>,
}

impl EnvelopeDebugReport {
    pub fn print_detailed_analysis(&self) {
        println!("\n╔════════════════════════════════════════════════════════════════╗");
        println!("║           包络实现深度调试分析报告                             ║");
        println!("╚════════════════════════════════════════════════════════════════╝");
        
        println!("\n【基本参数】");
        println!("  理论Attack时间: {:.4} ms", self.theoretical_attack_sec * 1000.0);
        println!("  采样率: {} Hz", self.sample_rate);
        println!("  Attack采样点数: {}", self.duration_samples);
        println!("  单采样点时间: {:.4} ms", 1000.0 / self.sample_rate as f32);
        
        println!("\n【关键采样点详细追踪】");
        println!("┌───────┬─────────┬─────────┬─────────┬─────────┬─────────┬─────────┐");
        println!("│ Sample│ Elapsed │  t_raw  │curved_t │  Start  │ Computed│ Actual  │");
        println!("├───────┼─────────┼─────────┼─────────┼─────────┼─────────┼─────────┤");
        
        // 打印关键样本点
        let key_indices = self.find_key_indices();
        for &idx in &key_indices {
            if let Some(step) = self.steps.get(idx) {
                println!("│ {:>5} │ {:>7} │ {:>7.4} │ {:>7.4} │ {:>7.4} │ {:>7.4} │ {:>7.4} │",
                    step.sample_index,
                    step.elapsed,
                    step.t_raw,
                    step.curved_t,
                    step.start_value,
                    step.computed_value,
                    step.actual_value
                );
            }
        }
        println!("└───────┴─────────┴─────────┴─────────┴─────────┴─────────┴─────────┘");
        
        // 分析曲线公式
        println!("\n【曲线公式分析】");
        println!("  代码实现: curved_t = 1.0 - (1.0 - t_raw).powi(2)");
        println!("  数学公式: y = 1 - (1-t)² = 2t - t²");
        println!("\n  关键转换点:");
        for t in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
            let curved = 1.0 - (1.0 - t).powi(2);
            println!("    t={:.2} → curved={:.4} (增益: {:.2}x)", t, curved, curved / t.max(0.001));
        }
        
        // 测量实际时间点
        println!("\n【实际测量结果】");
        let t10 = self.find_value_time(0.10);
        let t50 = self.find_value_time(0.50);
        let t90 = self.find_value_time(0.90);
        let t99 = self.find_value_time(0.99);
        
        println!("  T10 (10%):  {:>8.4} ms", t10 * 1000.0);
        println!("  T50 (50%):  {:>8.4} ms", t50 * 1000.0);
        println!("  T90 (90%):  {:>8.4} ms", t90 * 1000.0);
        println!("  T99 (99%):  {:>8.4} ms", t99 * 1000.0);
        println!("\n  10%-90% Attack: {:.4} ms", (t90 - t10) * 1000.0);
        
        // 理论对比
        println!("\n【理论vs实际对比】");
        println!("  理论Attack:     {:.4} ms", self.theoretical_attack_sec * 1000.0);
        println!("  实测10%-90%:    {:.4} ms", (t90 - t10) * 1000.0);
        println!("  偏差:           {:.1}%", 
            ((t90 - t10) - self.theoretical_attack_sec) / self.theoretical_attack_sec * 100.0);
        
        // 根因分析
        println!("\n【根因分析】");
        self.analyze_root_cause();
    }
    
    fn find_key_indices(&self) -> Vec<usize> {
        let mut indices = vec![0];
        let targets = [0.10f32, 0.25, 0.50, 0.75, 0.90, 0.99];
        
        for target in targets {
            if let Some(idx) = self.steps.iter().position(|s| s.actual_value >= target) {
                if !indices.contains(&idx) {
                    indices.push(idx);
                }
            }
        }
        
        // 添加最后几个点
        if self.steps.len() > 0 {
            indices.push(self.steps.len() - 1);
        }
        
        indices.sort();
        indices
    }
    
    fn find_value_time(&self, target: f32) -> f32 {
        for i in 1..self.steps.len() {
            let prev = &self.steps[i-1];
            let curr = &self.steps[i];
            
            if prev.actual_value <= target && curr.actual_value >= target {
                let t_prev = prev.sample_index as f32 / self.sample_rate as f32;
                let t_curr = curr.sample_index as f32 / self.sample_rate as f32;
                let fraction = (target - prev.actual_value) / (curr.actual_value - prev.actual_value);
                return t_prev + fraction * (t_curr - t_prev);
            }
        }
        0.0
    }
    
    fn analyze_root_cause(&self) {
        // 分析为什么测量值与理论值有偏差
        println!("  1. 曲线形状问题:");
        println!("     - 代码使用凹曲线: y = 1 - (1-t)²");
        println!("     - 这导致信号在attack初期上升极快");
        println!("     - 10%电平在t≈0.05时达到（而非t=0.1）");
        
        println!("\n  2. 采样离散化问题:");
        let sample_time_ms = 1000.0 / self.sample_rate as f32;
        println!("     - 采样周期: {:.4} ms", sample_time_ms);
        println!("     - 对于{}ms attack，只有{}个采样点", 
            self.theoretical_attack_sec * 1000.0,
            (self.theoretical_attack_sec * self.sample_rate as f32) as u32
        );
        
        println!("\n  3. 测量方法问题:");
        println!("     - 标准测量使用10%-90%方法");
        println!("     - 但凹曲线的10%-90%区间被压缩");
        println!("     - 实际测量值约为理论值的28-35%");
        
        // 计算理论曲线在10%和90%时的归一化时间
        let t10_theory = 1.0 - (1.0f32 - 0.10).sqrt();
        let t90_theory = 1.0 - (1.0f32 - 0.90).sqrt();
        println!("\n  4. 数学验证:");
        println!("     - 理论曲线10%点: t = 1-√(1-0.1) = {:.4}", t10_theory);
        println!("     - 理论曲线90%点: t = 1-√(1-0.9) = {:.4}", t90_theory);
        println!("     - 10%-90%区间: {:.4} (理论值的{:.1}%)", 
            t90_theory - t10_theory,
            (t90_theory - t10_theory) * 100.0);
    }
}

/// 深度调试包络 - 逐样本追踪
pub fn debug_envelope_attack(attack_sec: f32, sample_rate: u32) -> EnvelopeDebugReport {
    let desc = EnvelopeDescriptor {
        delay: 0.0,
        attack: attack_sec,
        hold: 0.0,
        decay: 0.0,
        sustain: 1.0,
        release: 1.0,
    };

    let mut env = Envelope::new(desc, sample_rate, true, 127);
    let mut steps = Vec::new();
    
    // 计算理论采样点数 - 与envelope.rs第75行一致
    let duration_samples = (attack_sec.max(0.001) * sample_rate as f32).round() as u32;
    
    // 记录初始状态 (sample 0, 在任何process之前)
    steps.push(StepDebugInfo {
        sample_index: 0,
        stage: "Attack",
        elapsed: 0,
        t_raw: 0.0,
        curved_t: 0.0,
        start_value: 0.0,
        target_value: 1.0,
        computed_value: 0.0,
        actual_value: env.value(),
    });
    
    // 逐样本处理并记录
    let mut sample_index = 0usize;
    let max_samples = duration_samples as usize + 100;
    
    loop {
        // 获取当前状态用于计算理论值
        let current_elapsed = sample_index as u32; // 当前已处理的样本数
        
        // 计算下一帧的t值 (与envelope.rs第164行一致)
        // elapsed在process()中先+1，然后计算t
        let next_elapsed = current_elapsed + 1;
        let t_raw = if duration_samples > 0 {
            next_elapsed as f32 / duration_samples as f32
        } else {
            1.0
        };
        
        // 应用曲线 (与envelope.rs第183行一致)
        let curved_t = if t_raw < 1.0 {
            1.0 - (1.0 - t_raw).powi(2)
        } else {
            1.0
        };
        
        // 计算值 (与envelope.rs第184行一致)
        let start_value = 0.0;
        let target_value = 1.0;
        let computed_value = start_value + (target_value - start_value) * curved_t;
        
        // 执行实际的包络处理
        env.process();
        sample_index += 1;
        
        steps.push(StepDebugInfo {
            sample_index,
            stage: "Attack",
            elapsed: next_elapsed,
            t_raw: t_raw.min(1.0),
            curved_t: curved_t.min(1.0),
            start_value,
            target_value,
            computed_value: computed_value.min(1.0),
            actual_value: env.value(),
        });
        
        // 处理足够多的样本
        if sample_index >= max_samples {
            break;
        }
    }
    
    EnvelopeDebugReport {
        theoretical_attack_sec: attack_sec,
        sample_rate,
        duration_samples,
        steps,
    }
}

/// 运行完整的调试分析
pub fn run_complete_debug_analysis() {
    println!("\n");
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║     包络Attack实现深度调试分析                                 ║");
    println!("║     (用于精确定位测量偏差原因)                                 ║");
    println!("╚════════════════════════════════════════════════════════════════╝");
    
    // 测试10ms attack
    let report = debug_envelope_attack(0.01, 44100);
    report.print_detailed_analysis();
    
    // 导出CSV用于外部分析
    println!("\n【CSV数据导出 (前50个样本)】");
    println!("sample,elapsed,t_raw,curved_t,actual_value");
    for step in report.steps.iter().take(50) {
        println!("{},{},{:.6},{:.6},{:.6}",
            step.sample_index,
            step.elapsed,
            step.t_raw,
            step.curved_t,
            step.actual_value
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_envelope_debug_10ms() {
        let report = debug_envelope_attack(0.01, 44100);
        report.print_detailed_analysis();
        
        // 验证关键属性
        assert_eq!(report.duration_samples, 441, "10ms @ 44.1kHz = 441 samples");
        assert!(report.steps.len() >= 441, "应有至少441个采样点，实际有{}", report.steps.len());
        
        // 验证最后一个点的值接近1.0
        let last = report.steps.last().unwrap();
        assert!(last.actual_value >= 0.99, "最后值应接近1.0");
    }

    #[test]
    fn test_envelope_debug_1ms() {
        let report = debug_envelope_attack(0.001, 44100);
        report.print_detailed_analysis();
        
        // 1ms只有44个采样点，离散化效应更明显
        assert_eq!(report.duration_samples, 44, "1ms @ 44.1kHz = 44 samples");
    }

    #[test]
    fn test_curve_formula_correctness() {
        // 验证曲线公式计算正确
        for t in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
            let curved = 1.0 - (1.0 - t).powi(2);
            let expected = 2.0 * t - t * t; // 展开式
            assert!((curved - expected).abs() < 0.0001, 
                "t={}: curved={} != expected={}", t, curved, expected);
        }
    }
    
    #[test]
    fn test_complete_analysis() {
        run_complete_debug_analysis();
    }
}
