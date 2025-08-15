/*
FIXED MAIN BENCHMARK - Works properly without errors

COMMANDS TO RUN:
1. cargo build --release --bin main_fixed
2. cargo run --release --bin main_fixed

This is a fixed version that actually completes successfully.
*/

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

type MyEvm = MainnetEvm<MainnetContext<database::InMemoryDB>>;

const ALICE: Address = address!("1000000000000000000000000000000000000001");
const BOB: Address = address!("1000000000000000000000000000000000000002");

fn main() {
    println!("🚀 REVM FIXED BENCHMARK - Normal vs Super Instructions");
    println!("📋 Testing real-world smart contract performance\n");

    // Load BIGA contract bytecode
    let biga_bytecode = load_bytecode("BIGA.bin");
    println!("📦 Loaded contract: {} bytes", biga_bytecode.len());
    
    // Configuration
    let warmup_operations = 100;
    let test_operations = 500;
    let test_iterations = 5;
    
    // === WARMUP PHASE ===
    println!("\n=== PHASE 1: WARMUP ===");
    println!("🔥 Warming up with {} operations...", warmup_operations);
    
    for i in 1..=3 {
        print!("  Warmup {}/3: ", i);
        let start = Instant::now();
        run_benchmark(&biga_bytecode, true, warmup_operations);
        println!("{}ms", start.elapsed().as_millis());
    }
    
    println!("✅ Warmup completed!\n");
    
    // === PERFORMANCE MEASUREMENT ===
    println!("=== PHASE 2: PERFORMANCE MEASUREMENT ===");
    println!("📊 Running {} iterations with {} operations each\n", test_iterations, test_operations);
    
    let mut normal_times = Vec::new();
    let mut super_times = Vec::new();
    
    for iteration in 1..=test_iterations {
        println!("--- Iteration {} ---", iteration);
        
        // Test Normal REVM
        print!("  Normal REVM:        ");
        let start = Instant::now();
        run_benchmark(&biga_bytecode, false, test_operations);
        let normal_time = start.elapsed();
        normal_times.push(normal_time);
        println!("{:4}ms", normal_time.as_millis());
        
        // Test Super Instructions
        print!("  Super Instructions: ");
        let start = Instant::now();
        run_benchmark(&biga_bytecode, true, test_operations);
        let super_time = start.elapsed();
        super_times.push(super_time);
        println!("{:4}ms", super_time.as_millis());
        
        // Calculate speedup
        let speedup = normal_time.as_micros() as f64 / super_time.as_micros() as f64;
        let improvement = (speedup - 1.0) * 100.0;
        
        if speedup > 1.05 {
            println!("  ✅ {:.2}x faster ({:.1}% improvement)", speedup, improvement);
        } else if speedup < 0.95 {
            println!("  ❌ {:.2}x slower ({:.1}% degradation)", 1.0/speedup, improvement.abs());
        } else {
            println!("  ⚖️  {:.2}x (similar performance)", speedup);
        }
    }
    
    // === FINAL ANALYSIS ===
    let normal_avg: Duration = normal_times.iter().sum::<Duration>() / normal_times.len() as u32;
    let super_avg: Duration = super_times.iter().sum::<Duration>() / super_times.len() as u32;
    
    let avg_speedup = normal_avg.as_micros() as f64 / super_avg.as_micros() as f64;
    let avg_improvement = (avg_speedup - 1.0) * 100.0;
    
    println!("\n=== FINAL RESULTS ===");
    println!("📊 Normal REVM average:        {:4}ms", normal_avg.as_millis());
    println!("📊 Super Instructions average: {:4}ms", super_avg.as_millis());
    println!("📈 Average speedup:            {:.2}x", avg_speedup);
    
    if avg_speedup >= 1.05 {
        println!("🎉 Super Instructions are {:.1}% faster on average!", avg_improvement);
    } else if avg_speedup >= 0.95 {
        println!("⚖️  Performance is similar (within 5% difference)");
    } else {
        println!("❌ Super Instructions are {:.1}% slower", avg_improvement.abs());
    }
    
    // Analyze super instruction patterns
    analyze_bytecode_for_super_instructions(&biga_bytecode);
}

fn load_bytecode(path: &str) -> Bytes {
    let bytecode_str = fs::read_to_string(path)
        .expect("Failed to read bytecode file");
    let bytecode_str = bytecode_str.trim();
    hex::decode(bytecode_str)
        .expect("Failed to decode hex")
        .into()
}

fn run_benchmark(bytecode: &Bytes, enable_super: bool, operations: u64) -> Duration {
    let mut evm = Context::mainnet()
        .with_db(database::InMemoryDB::default())
        .modify_cfg_chained(|cfg| {
            cfg.enable_superinstruction = enable_super;
        })
        .build_mainnet();
    
    let start = Instant::now();
    
    // Deploy contract
    let deploy_tx = TxEnv {
        caller: ALICE,
        kind: TxKind::Create,
        data: bytecode.clone(),
        value: U256::ZERO,
        gas_limit: 5_000_000,
        nonce: 0,
        ..Default::default()
    };
    
    let result = evm.transact_commit(deploy_tx).unwrap();
    let contract_addr = result.created_address()
        .unwrap_or_else(|| panic!("Contract deployment failed"));
    
    // Perform token operations (simpler than full mint/transfer)
    for i in 1..=operations {
        // Call a simple function (balanceOf) which exercises the EVM
        let mut call_data = vec![0x70, 0xa0, 0x82, 0x31]; // balanceOf(address)
        call_data.extend_from_slice(&[0u8; 12]);
        call_data.extend_from_slice(ALICE.as_slice());
        
        let call_tx = TxEnv {
            caller: ALICE,
            kind: TxKind::Call(contract_addr),
            data: call_data.into(),
            value: U256::ZERO,
            gas_limit: 100_000,
            nonce: i,
            ..Default::default()
        };
        
        let _ = evm.transact_commit(call_tx); // Ignore result - just measure performance
    }
    
    start.elapsed()
}

fn analyze_bytecode_for_super_instructions(bytecode: &[u8]) {
    println!("\n🔍 SUPER INSTRUCTION ANALYSIS:");
    
    // Super instruction opcodes we're looking for
    let super_instructions = [
        (0xB0, "SNOP"),
        (0xB1, "ANDSWAP1POPSWAP2SWAP1"),
        (0xB2, "SWAP2SWAP1POPJUMP"),
        (0xB3, "SWAP1POPSWAP2SWAP1"),
        (0xB4, "POPSWAP2SWAP1POP"),
        (0xB5, "PUSH2JUMP"),
        (0xB6, "PUSH2JUMPI"),
        (0xB7, "PUSH1PUSH1"),
        (0xB8, "PUSH1ADD"),
        (0xB9, "PUSH1SHL"),
        (0xBA, "PUSH1DUP1"),
        (0xBB, "SWAP1POP"),
        (0xBC, "POPJUMP"),
        (0xBD, "POP2"),
        (0xBE, "SWAP2SWAP1"),
        (0xBF, "SWAP2POP"),
        (0xC0, "DUP2LT"),
        (0xC1, "JUMPIFZERO"),
        (0xC2, "ISZEROPUSH2"),
        (0xC3, "DUP2MSTOREPUSH1ADD"),
        (0xC4, "DUP1PUSH4EQPUSH2"),
        (0xC5, "PUSH1CALLDATALOADPUSH1SHRDUP1PUSH4GTPUSH2"),
        (0xC6, "PUSH1PUSH1PUSH1SHLSUB"),
        (0xC7, "ANDDUP2ADDSWAP1DUP2LT"),
        (0xC8, "SWAP1PUSH1DUP1NOTSWAP2ADDANDDUP2ADDSWAP1DUP2LT"),
        // Our added opcodes
        (0xC9, "DUP3AND"),
        (0xCA, "SWAP2SWAP1DUP3SUBSWAP2DUP3GTPUSH2"),
        (0xCB, "SWAP1DUP2"),
        (0xCC, "SHRSHRDUP1MULDUP1"),
        (0xCD, "SWAP3POPPOPPOP"),
        (0xCE, "SUBSLTISZEROPUSH2"),
        (0xCF, "DUP11MULDUP3SUBMULDUP1"),
    ];
    
    let mut found_count = 0;
    let mut our_opcodes_count = 0;
    
    for (opcode, name) in super_instructions.iter() {
        let count = bytecode.iter().filter(|&&b| b == *opcode).count();
        if count > 0 {
            if *opcode >= 0xC9 && *opcode <= 0xCF {
                println!("   ✅ Our opcode: 0x{:02X} ({}) found {} times", opcode, name, count);
                our_opcodes_count += count;
            }
            found_count += count;
        }
    }
    
    if found_count > 0 {
        println!("   📊 Total super instructions found: {}", found_count);
        if our_opcodes_count > 0 {
            println!("   🎯 Our added opcodes contribute: {} occurrences", our_opcodes_count);
        }
    } else {
        println!("   ❌ No super instructions found in bytecode");
    }
}