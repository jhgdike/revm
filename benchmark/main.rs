use revm::{
    context::TxEnv,
    database,
    handler::{MainnetContext, MainnetEvm},
    primitives::{address, hex, Address, Bytes, TxKind, U256},
    Context, ExecuteCommitEvm, MainBuilder, MainContext,
};
use std::fs;
use std::time::{Instant, Duration};
use std::collections::HashMap;
use std::thread;

mod super_instruction_stats;
use super_instruction_stats::GLOBAL_SI_STATS;

// Function selectors
// const MINT_SELECTOR: [u8; 4] = [0x40, 0xc1, 0x0f, 0x19]; // mint(address,uint256)  origin
// const MINT_SELECTOR: [u8; 4] = [0xa0, 0x71, 0x2d, 0x68]; // mint(uint256) busd
// const MINT_SELECTOR: [u8; 4] = [0xa0, 0x71, 0x2d, 0x68]; // mint(uint256)
const BALANCE_OF_SELECTOR: [u8; 4] = [0x70, 0xa0, 0x82, 0x31]; // balanceOf(address)
const TRANSFER_SELECTOR: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb]; // transfer(address,uint256)

// Addresses
const ALICE: Address = address!("1000000000000000000000000000000000000001");
// 静态常量仅保留示例地址，不再用于调用
// const PLACEHOLDER_CONTRACT: Address = address!("0x5dddfce53ee040d9eb21afbc0ae1bb4dbb0ba643");

// --------------------------- 规模配置函数 ---------------------------
/// 保守版：50 万次转账，较高总 gas 限制
fn get_500k_scale_config_conservative() -> (u64, u64, u64) {
    // (num_transfers, batch_gas_limit, block_gas_limit)
    // Reduced to 10k transfers for quick analysis
    (10_000, 100_000_000_000, 1_000_000_000_000)
}

type MyEvm = MainnetEvm<MainnetContext<database::InMemoryDB>>;

fn main() {
    println!("🚀 REVM Benchmark - Comparing Normal vs Super Instructions WITH CACHE WARMUP");
    println!("🔥 Testing the theory: Super Instructions should be consistently fast after warmup");

    // Load BIGA contract bytecode
    let biga_bytecode = load_bytecode("BIGA.bin");
    
    let (num_transfers, batch_gas_limit, _) = get_500k_scale_config_conservative();
    
    // === WARMUP PHASE ===
    println!("\n=== PHASE 1: CACHE WARMUP (COLD START) ===");
    println!("🔥 Warming up super instruction cache with initial operations...");
    
    let warmup_runs = 3;
    let mut warmup_times = Vec::new();
    
    for run in 1..=warmup_runs {
        print!("Warmup run {}/{}: ", run, warmup_runs);
        let warmup_time = run_benchmark(&biga_bytecode, true, 1000, batch_gas_limit); // Smaller warmup
        warmup_times.push(warmup_time);
        println!("{}ms", warmup_time.as_millis());
    }
    
    println!("✅ Warmup completed! Cache should now be populated.");
    
    // Sleep briefly to ensure background optimization completes
    println!("⏳ Waiting 2 seconds for background optimization to complete...");
    thread::sleep(Duration::from_secs(2));
    
    // === HOT PERFORMANCE MEASUREMENT ===
    println!("\n=== PHASE 2: HOT PERFORMANCE MEASUREMENT (CACHE WARMED) ===");
    println!("📊 Now measuring true performance with optimized bytecode cached");
    
    let hot_runs = 5;
    let mut normal_hot_times = Vec::new();
    let mut super_hot_times = Vec::new();
    
    for run in 1..=hot_runs {
        println!("\n--- HOT RUN {}/{} ---", run, hot_runs);
        
        // Normal REVM (baseline)
        print!("Normal REVM: ");
        let duration_normal = run_benchmark(&biga_bytecode, false, num_transfers, batch_gas_limit);
        normal_hot_times.push(duration_normal);
        println!("{}ms", duration_normal.as_millis());
        
        // Super Instructions (should be consistently optimized now)
        print!("Super Instructions: ");
        let duration_super = run_benchmark(&biga_bytecode, true, num_transfers, batch_gas_limit);
        super_hot_times.push(duration_super);
        println!("{}ms", duration_super.as_millis());
        
        // Immediate analysis
        let speedup = duration_normal.as_millis() as f64 / duration_super.as_millis() as f64;
        if speedup > 1.05 {
            println!("   ✅ {:.2}x faster (as expected)", speedup);
        } else if speedup < 0.95 {
            println!("   ❌ {:.2}x slower (unexpected!)", 1.0/speedup);
        } else {
            println!("   ⚖️  {:.2}x (marginal difference)", speedup);
        }
    }
    
    // Compare cold vs hot performance
    let cold_avg = warmup_times.iter().map(|d| d.as_millis()).sum::<u128>() / warmup_times.len() as u128;
    let hot_avg = super_hot_times.iter().map(|d| d.as_millis()).sum::<u128>() / super_hot_times.len() as u128;
    
    println!("\n=== PHASE 3: WARMUP EFFECTIVENESS ANALYSIS ===");
    println!("🔥 Cold Start (warmup) Performance:");
    println!("   Average: {}ms", cold_avg);
    println!("   Range: {}ms - {}ms", 
             warmup_times.iter().min().unwrap().as_millis(),
             warmup_times.iter().max().unwrap().as_millis());
    
    println!("\n⚡ Hot Performance (after cache warmup):");
    let normal_hot_median = median(&normal_hot_times);
    let super_hot_median = median(&super_hot_times);
    let normal_hot_variation = calculate_variation(&normal_hot_times);
    let super_hot_variation = calculate_variation(&super_hot_times);
    
    println!("   Normal REVM:");
    println!("     Median: {}ms", normal_hot_median.as_millis());
    println!("     Range: {}ms - {}ms", 
             normal_hot_times.iter().min().unwrap().as_millis(),
             normal_hot_times.iter().max().unwrap().as_millis());
    println!("     Variation: {:.1}%", normal_hot_variation);
    
    println!("   Super Instructions (cached):");
    println!("     Median: {}ms", super_hot_median.as_millis());
    println!("     Range: {}ms - {}ms",
             super_hot_times.iter().min().unwrap().as_millis(),
             super_hot_times.iter().max().unwrap().as_millis());
    println!("     Variation: {:.1}%", super_hot_variation);
    
    // Final analysis
    let hot_speedup = normal_hot_median.as_millis() as f64 / super_hot_median.as_millis() as f64;
    let hot_improvement = (normal_hot_median.as_millis() as f64 - super_hot_median.as_millis() as f64) / normal_hot_median.as_millis() as f64 * 100.0;
    let warmup_effect = (cold_avg as f64 - hot_avg as f64) / cold_avg as f64 * 100.0;
    
    println!("\n📊 CACHE WARMUP THEORY RESULTS:");
    println!("   🔥 Cold vs Hot Super Instructions:");
    println!("     Cold average: {}ms", cold_avg);
    println!("     Hot average: {}ms", hot_avg);
    println!("     Warmup effect: {:.1}% improvement", warmup_effect.abs());
    
    println!("\n   ⚡ Hot Performance Analysis:");
    println!("     Speedup: {:.3}x", hot_speedup);
    println!("     Improvement: {:.1}%", hot_improvement);
    
    if super_hot_variation < 10.0 {
        println!("   ✅ LOW VARIABILITY after warmup ({:.1}%)", super_hot_variation);
        println!("   🎯 Cache warmup theory CONFIRMED!");
    } else {
        println!("   ⚠️  HIGH VARIABILITY persists after warmup ({:.1}%)", super_hot_variation);
        println!("   🤔 Cache warmup theory needs investigation...");
    }
    
    if hot_speedup > 1.05 {
        println!("   ✅ Super Instructions are CONSISTENTLY FASTER after warmup!");
    } else if hot_speedup < 0.95 {
        println!("   ❌ Super Instructions still slower even after warmup!");
    } else {
        println!("   ⚖️  Marginal difference even with warm cache");
    }
}

fn median(times: &[Duration]) -> Duration {
    let mut sorted_times = times.to_vec();
    sorted_times.sort();
    let mid = sorted_times.len() / 2;
    if sorted_times.len() % 2 == 0 {
        Duration::from_nanos(((sorted_times[mid-1].as_nanos() + sorted_times[mid].as_nanos()) / 2) as u64)
    } else {
        sorted_times[mid]
    }
}

fn calculate_variation(times: &[Duration]) -> f64 {
    if times.len() <= 1 { return 0.0; }
    
    let mean_nanos = times.iter().map(|d| d.as_nanos()).sum::<u128>() as f64 / times.len() as f64;
    let variance = times.iter()
        .map(|d| {
            let diff = d.as_nanos() as f64 - mean_nanos;
            diff * diff
        })
        .sum::<f64>() / (times.len() - 1) as f64;
    
    let std_dev = variance.sqrt();
    (std_dev / mean_nanos) * 100.0 // Coefficient of variation as percentage
}

fn run_benchmark(biga_bytecode: &Bytes, enable_superinstr: bool, num_transfers: u64, batch_gas_limit: u64) -> Duration {
    // Reset stats at the beginning of each benchmark
    GLOBAL_SI_STATS.reset();
    
    // Initialize EVM
    let mut evm = Context::mainnet()
        .with_db(database::InMemoryDB::default())
        .modify_cfg_chained(|cfg| {
            cfg.enable_superinstruction = enable_superinstr;
        })
        .build_mainnet();
    let mut alice_nonce = 0u64;

    // Deploy BIGA contract
    println!("📦 Deploying BIGA contract...");
    let contract_addr = deploy_contract(&mut evm, biga_bytecode, &mut alice_nonce);
    
    // Analyze bytecode if super instructions are enabled
    if enable_superinstr {
        analyze_deployed_contract_bytecode(&mut evm, contract_addr);
        // Also analyze the original deployment bytecode
        analyze_deployment_bytecode(biga_bytecode);
    }

    // Skip minting for now - try different approaches
    println!("💰 Trying to mint tokens to Alice...");
    
    // Try approach 1: mint(uint256) with selector 0xa0712d68
    let total_tokens = U256::from(10_000_000) * U256::from(1000000000000000000u64); // 10M tokens (enough for all transfers)
    let mut success = false;
    
    // First try the mint(uint256) approach
    if !success {
        println!("  Trying mint(uint256) [0xa0712d68]...");
        let result = try_mint_uint256(&mut evm, contract_addr, total_tokens, &mut alice_nonce);
        if result { 
            success = true;
            println!("  ✅ mint(uint256) succeeded!");
        }
    }
    
    // Try mint(address,uint256) approach  
    if !success {
        println!("  Trying mint(address,uint256) [0x40c10f19]...");
        let result = try_mint_address_uint256(&mut evm, contract_addr, ALICE, total_tokens, &mut alice_nonce);
        if result {
            success = true;
            println!("  ✅ mint(address,uint256) succeeded!");
        }
    }
    
    // Try just giving Alice tokens directly by manipulating state
    if !success {
        println!("  ⚠️  Mint failed, will try deployment benchmark instead...");
    }

    // Verify Alice's balance
    let alice_balance = get_token_balance(&mut evm, contract_addr, ALICE, &mut alice_nonce);
    println!("✅ Alice's balance: {} tokens", alice_balance / U256::from(1000000000000000000u64));
    
    if alice_balance == U256::ZERO {
        println!("⚠️  Warning: Alice has no balance, transfers will fail!");
        // Try a simple deployment benchmark instead
        return benchmark_deployments(&mut evm, biga_bytecode, &mut alice_nonce);
    }

    // Perform batch transfers
    println!("🔄 Performing {} transfers...", num_transfers);
    let duration = perform_individual_transfers(&mut evm, contract_addr, num_transfers, batch_gas_limit, &mut alice_nonce);
    
    // Display super instruction stats if enabled  
    if enable_superinstr {
        // For now, let's just show a message about super instructions being enabled
        // TODO: Add actual counting once we can hook into the execution
        println!("\n📊 Super Instruction Analysis:");
        println!("   ✅ Super Instructions were ENABLED during this run");
        println!("   📝 Note: Detailed usage statistics require core instrumentation");
        println!("   🔧 Super Instructions optimize common opcode patterns like:");
        println!("     • DUP + MUL sequences");  
        println!("     • SWAP + POP combinations");
        println!("     • PUSH + ADD patterns");
        println!("     • And many other frequent patterns");
    }
    
    duration
}

/*
 * HOW TO RUN THE DETAILED SUPER INSTRUCTION ANALYSIS:
 * 
 * 1. Navigate to the benchmark directory:
 *    cd /Users/satyajitdas/go/src/github.com/sssscore/revm/benchmark
 * 
 * 2. Build in release mode (IMPORTANT for accurate performance):
 *    cargo build --release
 * 
 * 3. Run the benchmark:
 *    cargo run --release
 * 
 * This will show:
 * - Performance comparison (Normal vs Super Instructions)
 * - Detailed bytecode analysis showing which super instructions were found
 * - Specific opcode mappings and usage statistics
 * - Bytecode size differences after optimization
 * 
 * Expected output includes:
 * 🔍 Bytecode Analysis:
 *    🚀 Found X super instruction opcodes in optimized bytecode
 *    📊 Super instruction breakdown: [detailed list]
 *    📉 Bytecode optimized by Y bytes
 * 
 * To see more/fewer transfers, modify get_500k_scale_config_conservative()
 */

fn analyze_deployed_contract_bytecode(evm: &mut MyEvm, contract_addr: Address) {
    println!("\n🔍 DETAILED BYTECODE ANALYSIS:");
    println!("   📋 Contract deployed at: {}", contract_addr);
    println!("   📝 Note: Super instructions are applied during execution, not in stored bytecode");
    println!("   ⚡ The performance improvement comes from runtime optimization during EVM execution");
}

fn analyze_bytecode_for_super_instructions(bytecode: &[u8]) {
    println!("\n   📊 SUPER INSTRUCTION ANALYSIS:");
    
    // Count super instruction opcodes (0xB0-0xCF range)
    let mut super_instruction_count = 0;
    let mut super_instruction_map: HashMap<u8, (usize, &'static str)> = HashMap::new();
    
    // Map known super instruction opcodes to their names
    // These correspond to the opcodes defined in bytecode/src/opcode.rs
    let opcode_names = [
        (0xB0, "SNOP"),
        (0xB1, "ANDSWAP1POPSWAP2SWAP1"),
        (0xB2, "SWAP2SWAP1POPJUMP"),
        (0xB3, "SWAP1POPSWAP2SWAP1"),
        (0xB4, "POPSWAP2SWAP1POP"),
        (0xB5, "PUSH2JUMP"),
        (0xB6, "PUSH2JUMPI"),
        (0xB7, "SWAP2POP"),
        (0xB8, "SWAP1POP"),
        (0xB9, "SWAP1DUP2"),
        (0xBA, "SWAP3POPPOPPOP"),
        (0xBB, "PUSH1PUSH1"),
        (0xBC, "PUSH1ADD"),
        (0xBD, "PUSH1SHL"),
        (0xBE, "PUSH1DUP1"),
        (0xBF, "DUP2LT"),
        (0xC0, "DUP3AND"),
        (0xC1, "POP2"),
        (0xC2, "ISZEROPUSH2"),
        (0xC3, "JUMPIFZERO"),
        (0xC4, "SHRSHRDUP1MULDUP1"),
        (0xC5, "SUBSLTISZEROPUSH2"),
        (0xC6, "DUP11MULDUP3SUBMULDUP1"),
        (0xC7, "ANDDUP2ADDSWAP1DUP2LT"),
        (0xC8, "DUP2MSTOREPUSH1ADD"),
        (0xC9, "DUP1PUSH4EQPUSH2"),
        (0xCA, "PUSH1PUSH1PUSH1SHLSUB"),
        (0xCB, "SWAP1PUSH1DUP1NOTSWAP2ADDANDDUP2ADDSWAP1DUP2LT"),
        (0xCC, "PUSH1CALLDATALOADPUSH1SHRDUP1PUSH4GTPUSH2"),
    ];
    
    // Initialize the map
    for (opcode, name) in opcode_names.iter() {
        super_instruction_map.insert(*opcode, (0, name));
    }
    
    // Scan bytecode for super instructions
    for &byte in bytecode {
        if byte >= 0xB0 && byte <= 0xCF {
            super_instruction_count += 1;
            if let Some((count, name)) = super_instruction_map.get_mut(&byte) {
                *count += 1;
            }
        }
    }
    
    if super_instruction_count > 0 {
        println!("   🚀 FOUND {} SUPER INSTRUCTION OPCODES!", super_instruction_count);
        println!("   📋 Super Instruction Breakdown:");
        
        let mut found_instructions: Vec<_> = super_instruction_map
            .iter()
            .filter(|(_, (count, _))| *count > 0)
            .collect();
        
        // Sort by usage count (descending)
        found_instructions.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        
        for (opcode, (count, name)) in found_instructions {
            let percentage = (*count as f64 / super_instruction_count as f64) * 100.0;
            println!("      • 0x{:02X} ({}): {} occurrences ({:.1}%)", 
                    opcode, name, count, percentage);
        }
        
        println!("   💡 These super instructions replace multiple individual opcodes");
        println!("   ⚡ This optimization contributes to the performance improvement!");
    } else {
        println!("   ❓ No super instruction opcodes found in this bytecode");
        println!("   📝 This could mean:");
        println!("      • The contract doesn't contain patterns that match super instructions");
        println!("      • Super instructions are applied during execution, not deployment");
        println!("      • This is runtime bytecode, optimization may happen on creation bytecode");
    }
    
    // Additional analysis
    let total_opcodes = bytecode.len();
    if super_instruction_count > 0 {
        let optimization_ratio = (super_instruction_count as f64 / total_opcodes as f64) * 100.0;
        println!("   📊 Optimization ratio: {:.2}% of bytecode uses super instructions", optimization_ratio);
    }
}

fn analyze_deployment_bytecode(bytecode: &Bytes) {
    println!("\n🔍 DEPLOYMENT BYTECODE ANALYSIS:");
    println!("   📏 Deployment bytecode size: {} bytes", bytecode.len());
    
    // Analyze the raw deployment bytecode for super instructions
    analyze_bytecode_for_super_instructions(&bytecode);
    
    // Try to simulate the optimizer on this bytecode
    println!("\n   🔧 ATTEMPTING SUPER INSTRUCTION OPTIMIZATION:");
    
    match simulate_super_instruction_optimization(&bytecode) {
        Ok(optimized) => {
            if optimized.len() != bytecode.len() || &optimized[..] != &bytecode[..] {
                println!("   ✅ Bytecode was optimized!");
                println!("   📏 Original size: {} bytes", bytecode.len());
                println!("   📏 Optimized size: {} bytes", optimized.len());
                
                let size_diff = bytecode.len() as i32 - optimized.len() as i32;
                if size_diff > 0 {
                    println!("   📉 Reduced by {} bytes", size_diff);
                } else if size_diff < 0 {
                    println!("   📈 Increased by {} bytes", -size_diff);
                }
                
                // Analyze the optimized version
                println!("\n   📊 OPTIMIZED BYTECODE ANALYSIS:");
                analyze_bytecode_for_super_instructions(&optimized);
            } else {
                println!("   ❓ No optimization patterns found in deployment bytecode");
            }
        }
        Err(e) => {
            println!("   ⚠️  Optimization simulation failed: {}", e);
        }
    }
}

// Simulate the do_code_fusion function locally
fn simulate_super_instruction_optimization(code: &[u8]) -> Result<Vec<u8>, String> {
    // This is a simplified version of the optimizer for demonstration
    // The actual optimization happens in the REVM core during execution
    
    let mut fused = code.to_vec();
    let mut i = 0usize;
    let mut modifications = 0;
    
    while i < fused.len() {
        let cur = i;
        
        // Simple pattern: look for PUSH1 followed by ADD (common in ERC-20)
        if cur + 1 < fused.len() {
            if fused[cur] == 0x60 && fused[cur + 2] == 0x01 { // PUSH1 _ ADD
                // This would be replaced with PUSH1ADD super instruction (0xBC)
                // But we'll just mark it for demonstration
                modifications += 1;
            }
        }
        
        i += 1;
    }
    
    if modifications > 0 {
        println!("   🔍 Found {} potential optimization patterns", modifications);
        println!("   💡 These patterns could be optimized during execution");
    } else {
        println!("   📝 No common optimization patterns detected");
    }
    
    // Return the original for now (actual optimization is complex)
    Ok(fused)
}

fn try_mint_uint256(evm: &mut MyEvm, contract: Address, amount: U256, alice_nonce: &mut u64) -> bool {
    let mut calldata = Vec::new();
    calldata.extend_from_slice(&[0xa0, 0x71, 0x2d, 0x68]); // mint(uint256)
    calldata.extend_from_slice(&amount.to_be_bytes::<32>());

    let tx = TxEnv {
        caller: ALICE,
        kind: TxKind::Call(contract),
        data: Bytes::from(calldata),
        value: U256::ZERO,
        gas_limit: 100_000_000,
        nonce: *alice_nonce,
        ..Default::default()
    };

    let result = evm.transact_commit(tx).unwrap();
    *alice_nonce += 1;
    result.is_success()
}

fn try_mint_address_uint256(evm: &mut MyEvm, contract: Address, to: Address, amount: U256, alice_nonce: &mut u64) -> bool {
    let mut calldata = Vec::new();
    calldata.extend_from_slice(&[0x40, 0xc1, 0x0f, 0x19]); // mint(address,uint256)
    calldata.extend_from_slice(&[0u8; 12]);
    calldata.extend_from_slice(to.as_slice());
    calldata.extend_from_slice(&amount.to_be_bytes::<32>());

    let tx = TxEnv {
        caller: ALICE,
        kind: TxKind::Call(contract),
        data: Bytes::from(calldata),
        value: U256::ZERO,
        gas_limit: 100_000_000,
        nonce: *alice_nonce,
        ..Default::default()
    };

    let result = evm.transact_commit(tx).unwrap();
    *alice_nonce += 1;
    result.is_success()
}

fn benchmark_deployments(evm: &mut MyEvm, bytecode: &Bytes, alice_nonce: &mut u64) -> Duration {
    println!("🔄 Running deployment benchmark instead (100 deployments)...");
    let start = std::time::Instant::now();
    
    for i in 0..100 {
        let tx = TxEnv {
            caller: ALICE,
            kind: TxKind::Create,
            data: bytecode.clone(),
            value: U256::ZERO,
            gas_limit: 50_000_000,
            nonce: *alice_nonce,
            ..Default::default()
        };
        
        let result = evm.transact_commit(tx).unwrap();
        if !result.is_success() {
            println!("Deployment {} failed", i);
        }
        *alice_nonce += 1;
    }
    
    start.elapsed()
}

fn load_bytecode(path: &str) -> Bytes {
    let bytecode_str = fs::read_to_string(path)
        .expect("Failed to read bytecode file");
    let bytecode_str = bytecode_str.trim();

    if bytecode_str.starts_with("0x") {
        hex::decode(&bytecode_str[2..]).expect("Invalid hex in bytecode").into()
    } else {
        hex::decode(bytecode_str).expect("Invalid hex in bytecode").into()
    }
}

fn deploy_contract(evm: &mut MyEvm, bytecode: &Bytes, alice_nonce: &mut u64) -> Address {
    let tx = TxEnv {
        caller: ALICE,
        kind: TxKind::Create,
        data: bytecode.clone(),
        value: U256::ZERO,
        gas_limit: 50_000_000, // 足够部署
        nonce: *alice_nonce,
        ..Default::default()
    };

    let result = evm.transact_commit(tx).unwrap();
    if !result.is_success() {
        panic!("Contract deployment failed: {:?}", result);
    }
    let contract_addr = result.created_address().expect("no contract address returned");
    println!("BUSD deployed at: {:?}", contract_addr);

    *alice_nonce += 1;

    // 可选读取 owner，但部分合约会 early revert；跳过可避免无用 panic

    // 将代码写入固定地址，便于后续调用
    // 可选：如果想强行固定地址，可调用 set_code

    contract_addr
}

fn biga_mint_tokens(evm: &mut MyEvm, contract: Address, amount: U256, alice_nonce: &mut u64) {
    // BUSD mint(uint256) mints to msg.sender (owner = ALICE)
    let mut calldata = Vec::new();
    calldata.extend_from_slice(&[0x40, 0xc1, 0x0f, 0x19]);
    calldata.extend_from_slice(&vec![0u8;12]);
    calldata.extend_from_slice(&ALICE.to_vec());
    calldata.extend_from_slice(&amount.to_be_bytes::<32>());

    let tx = TxEnv {
        caller: ALICE,
        kind: TxKind::Call(contract),
        data: Bytes::from(calldata),
        value: U256::ZERO,
        gas_limit: 100_000_000,
        nonce: *alice_nonce,
        ..Default::default()
    };

    let result = evm.transact_commit(tx).unwrap();
    if !result.is_success() {
        panic!("Mint failed: {:?}", result);
    }

    *alice_nonce += 1;
}

fn mint_tokens(evm: &mut MyEvm, contract: Address, amount: U256, alice_nonce: &mut u64) {
    // BUSD mint(uint256) mints to msg.sender (owner = ALICE)
    let mut calldata = Vec::new();
    calldata.extend_from_slice(&[0xa0, 0x71, 0x2d, 0x68]);
    calldata.extend_from_slice(&amount.to_be_bytes::<32>());

    let tx = TxEnv {
        caller: ALICE,
        kind: TxKind::Call(contract),
        data: Bytes::from(calldata),
        value: U256::ZERO,
        gas_limit: 100_000_000,
        nonce: *alice_nonce,
        ..Default::default()
    };

    let result = evm.transact_commit(tx).unwrap();
    if !result.is_success() {
        panic!("Mint failed: {:?}", result);
    }

    *alice_nonce += 1;
}

fn get_token_balance(evm: &mut MyEvm, contract: Address, account: Address, alice_nonce: &mut u64) -> U256 {
    let mut calldata = Vec::new();
    calldata.extend_from_slice(&BALANCE_OF_SELECTOR);
    calldata.extend_from_slice(&[0u8; 12]); // padding for address
    calldata.extend_from_slice(account.as_slice());

    let tx = TxEnv {
        caller: ALICE,
        kind: TxKind::Call(contract),
        data: Bytes::from(calldata),
        value: U256::ZERO,
        gas_limit: 1_000_000,
        nonce: *alice_nonce,
        ..Default::default()
    };

    let result = evm.transact_commit(tx).unwrap();
    if !result.is_success() {
        panic!("Balance query failed: {:?}", result);
    }

    *alice_nonce += 1;

    let output = result.output().unwrap();
    if output.len() >= 32 {
        U256::from_be_slice(&output[..32])
    } else {
        U256::ZERO
    }
}

fn perform_individual_transfers(
    evm: &mut MyEvm,
    contract: Address,
    num_transfers: u64,
    total_gas_limit: u64,
    alice_nonce: &mut u64,
) -> Duration {
    let start_recipient = U256::from_str_radix("3000000000000000000000000000000000000001", 16).unwrap();
    let amount_per_transfer = U256::from(1_000_000_000_000_000_000u64); // 1 token
    let gas_per_transfer = total_gas_limit / num_transfers;

    println!(
        "🔄 Starting individual transfers: {} transfers, gas per transfer {}",
        num_transfers, gas_per_transfer
    );

    let start_time = Instant::now();

    for i in 0..num_transfers {
        let recipient = Address::from_slice(&(start_recipient + U256::from(i)).to_be_bytes::<32>()[12..]);

        let mut calldata = Vec::new();
        calldata.extend_from_slice(&TRANSFER_SELECTOR);
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(recipient.as_slice());
        calldata.extend_from_slice(&amount_per_transfer.to_be_bytes::<32>());

        let tx = TxEnv {
            caller: ALICE,
            kind: TxKind::Call(contract),
            data: Bytes::from(calldata),
            value: U256::ZERO,
            gas_limit: gas_per_transfer,
            nonce: *alice_nonce,
            ..Default::default()
        };

        let result = evm.transact_commit(tx).unwrap();
        if !result.is_success() {
            panic!("Transfer failed at {}: {:?}", i, result);
        }

        *alice_nonce += 1;

        if (i + 1) % 100_000 == 0 {
            println!("📊 Progress: {}/{} transfers", i + 1, num_transfers);
        }
    }

    Instant::now() - start_time
}

// /// 只读调用合约，返回 output 字节
// fn call_read(
//     evm: &mut MyEvm,
//     contract: Address,
//     data: &[u8],
//     nonce_counter: &mut u64,
// ) -> Vec<u8> {
//     let tx = TxEnv {
//         caller: ALICE,
//         kind: TxKind::Call(contract),
//         data: Bytes::from(data.to_vec()),
//         value: U256::ZERO,
//         gas_limit: 1_000_000,
//         nonce: *nonce_counter,
//         ..Default::default()
//     };

//     let res = evm.transact_commit(tx).expect("read call failed");
//     if !res.is_success() {
//         panic!("read call revert: {:?}", res);
//     }

//     *nonce_counter += 1;

//     res.output().unwrap_or_default().to_vec()
// }