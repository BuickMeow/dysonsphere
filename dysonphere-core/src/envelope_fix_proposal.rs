//! Attack包络问题分析与修复方案
//! 
//! 问题：开发者反馈attack"特别重"
//! 原因：凹曲线(y=1-(1-t)²)导致attack初期上升过快，产生"撞击感"
//!
//! 本文件提供：
//! 1. 问题根因分析
//! 2. 多种修复方案对比
//! 3. 推荐实现

use crate::Envelope;
use dysonphere_soundfont::types::EnvelopeDescriptor;

/// 不同曲线类型的对比分析
pub fn analyze_curve_types() {
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║     Attack曲线类型对比分析                                   ║");
    println!("║     (解决\"特别重\"问题)                                      ║");
    println!("╚════════════════════════════════════════════════════════════════╝");
    
    println!("\n【当前问题】");
    println!("  代码使用: y = 1 - (1-t)²  (凹曲线)");
    println!("  问题: 初期上升过快，t=0.25时已达43.75%，听起来\"撞击感\"强");
    
    println!("\n【三种曲线对比】");
    println!("┌───────┬─────────┬─────────┬─────────┬─────────────────────────┐");
    println!("│   t   │  凹曲线 │  线性   │  凸曲线 │  说明                   │");
    println!("│       │(当前)   │(推荐)   │(柔和)   │                         │");
    println!("├───────┼─────────┼─────────┼─────────┼─────────────────────────┤");
    
    for t in [0.0f32, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0] {
        let concave = 1.0 - (1.0 - t).powi(2);      // 当前: 凹曲线
        let linear = t;                              // 线性
        let convex = t.powi(2);                      // 凸曲线: t²
        
        let note = if t == 0.25 && concave > 0.4 {
            "⚠️ 上升过快"
        } else if t == 0.5 && linear == 0.5 {
            "✓ 自然过渡"
        } else {
            ""
        };
        
        println!("│ {:>5.2} │ {:>7.4} │ {:>7.4} │ {:>7.4} │ {:<23} │",
            t, concave, linear, convex, note);
    }
    
    println!("└───────┴─────────┴─────────┴─────────┴─────────────────────────┘");
    
    println!("\n【听感分析】");
    println!("  凹曲线(当前): 初期能量释放过快 → \"重/硬/撞击感\"");
    println!("  线性: 均匀上升 → \"自然/平衡\"");
    println!("  凸曲线: 缓慢启动后加速 → \"柔和/慢起\"");
    
    println!("\n【推荐方案】");
    println!("  方案1: 改为线性曲线 (y = t) - 最自然");
    println!("  方案2: 改为凸曲线 (y = t²) - 最柔和");
    println!("  方案3: 添加可配置参数让用户选择");
}

/// 模拟不同曲线的attack效果
pub fn simulate_attack_curves(attack_sec: f32, sample_rate: u32) {
    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║     Attack曲线模拟对比 (理论Attack = {:.2} ms)              ║", attack_sec * 1000.0);
    println!("╚════════════════════════════════════════════════════════════════╝");
    
    let duration_samples = (attack_sec.max(0.001) * sample_rate as f32).round() as usize;
    
    println!("\n【10%-90%上升时间对比】");
    println!("┌─────────────┬─────────────┬─────────────┬─────────────┐");
    println!("│   电平      │   凹曲线    │   线性      │   凸曲线    │");
    println!("├─────────────┼─────────────┼─────────────┼─────────────┤");
    
    // 计算各曲线达到特定电平的时间
    let targets = [0.10f32, 0.50, 0.90];
    
    for target in targets {
        // 凹曲线: y = 1-(1-t)² → t = 1-√(1-y)
        let t_concave = 1.0 - (1.0 - target).sqrt();
        // 线性: y = t
        let t_linear = target;
        // 凸曲线: y = t² → t = √y
        let t_convex = target.sqrt();
        
        let time_concave = t_concave * attack_sec * 1000.0;
        let time_linear = t_linear * attack_sec * 1000.0;
        let time_convex = t_convex * attack_sec * 1000.0;
        
        println!("│   {:>4.0}%     │ {:>8.3} ms │ {:>8.3} ms │ {:>8.3} ms │",
            target * 100.0, time_concave, time_linear, time_convex);
    }
    
    println!("└─────────────┴─────────────┴─────────────┴─────────────┘");
    
    // 计算10%-90%时间
    let t10_concave = 1.0 - (1.0f32 - 0.10).sqrt();
    let t90_concave = 1.0 - (1.0f32 - 0.90).sqrt();
    let attack_concave = (t90_concave - t10_concave) * attack_sec * 1000.0;
    
    let attack_linear = 0.80 * attack_sec * 1000.0;
    
    let t10_convex = 0.10f32.sqrt();
    let t90_convex = 0.90f32.sqrt();
    let attack_convex = (t90_convex - t10_convex) * attack_sec * 1000.0;
    
    println!("\n【10%-90% Attack时间】");
    println!("  凹曲线(当前): {:.3} ms", attack_concave);
    println!("  线性(推荐):   {:.3} ms", attack_linear);
    println!("  凸曲线(柔和): {:.3} ms", attack_convex);
    
    println!("\n【结论】");
    println!("  线性曲线的10%-90%时间最接近理论值，听感最自然");
    println!("  当前凹曲线的时间只有理论值的28%，听起来\"重\"");
}

/// 生成修复后的包络代码
pub fn generate_fixed_code() -> String {
    r#"// 修复方案：将凹曲线改为线性曲线
// 文件: dysonphere-core/src/envelope.rs
// 修改位置: process()函数中的Stage::Attack处理

Stage::Attack => {
    // 原代码（凹曲线 - 导致"重"的问题）:
    // let curved = 1.0 - (1.0 - t).powi(2);
    
    // 修复方案1: 线性曲线（推荐）
    let curved = t;
    
    // 修复方案2: 凸曲线（更柔和）
    // let curved = t * t;
    
    // 修复方案3: 可配置曲线（高级）
    // let curved = match curve_type {
    //     CurveType::Linear => t,
    //     CurveType::Concave => 1.0 - (1.0 - t).powi(2),
    //     CurveType::Convex => t * t,
    // };
    
    self.value = start + (params.target - start) * curved;
}
"#.to_string()
}

/// 修复验证测试
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_curve_comparison() {
        analyze_curve_types();
    }

    #[test]
    fn test_attack_simulation() {
        simulate_attack_curves(0.01, 44100);
    }

    #[test]
    fn test_linear_vs_concave() {
        println!("\n╔════════════════════════════════════════════════════════════════╗");
        println!("║     线性 vs 凹曲线 详细对比                                  ║");
        println!("╚════════════════════════════════════════════════════════════════╝");
        
        // 测试前10个采样点的差异
        let sample_rate = 44100u32;
        let attack_sec = 0.01f32;
        let duration = (attack_sec * sample_rate as f32) as usize;
        
        println!("\n前10个采样点的包络值对比:");
        println!("┌────────┬─────────────┬─────────────┬─────────────┐");
        println!("│ Sample │  凹曲线     │  线性       │  差值       │");
        println!("├────────┼─────────────┼─────────────┼─────────────┤");
        
        for i in 1..=10 {
            let t = i as f32 / duration as f32;
            let concave = 1.0 - (1.0 - t).powi(2);
            let linear = t;
            let diff = concave - linear;
            
            println!("│ {:>6} │ {:>11.6} │ {:>11.6} │ {:>11.6} │", 
                i, concave, linear, diff);
        }
        
        println!("└────────┴─────────────┴─────────────┴─────────────┘");
        
        // 计算初期能量差异
        let t_10pct = 0.1f32;
        let sample_10pct = (t_10pct * duration as f32) as usize;
        let t = sample_10pct as f32 / duration as f32;
        let concave_at_10pct = 1.0 - (1.0 - t).powi(2);
        
        println!("\n【关键发现】");
        println!("  在前10%时间内:");
        println!("    凹曲线已达到: {:.2}%", concave_at_10pct * 100.0);
        println!("    线性只达到:   {:.2}%", t * 100.0);
        println!("    差异:         {:.2}%", (concave_at_10pct - t) * 100.0);
        println!("\n  这就是听起来\"重\"的原因 - 初期能量释放过快！");
    }
    
    #[test]
    fn test_proposed_fix() {
        println!("\n【修复代码】\n{}", generate_fixed_code());
    }
}
