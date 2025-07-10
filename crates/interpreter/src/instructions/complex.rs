use crate::{
    gas,
    interpreter_types::{InterpreterTypes, StackTr, Jumps, Immediates, MemoryTr, InputsTr},
    Interpreter,
};
use primitives::{U256, B256};
use crate::InstructionResult;
use crate::interpreter_types::LoopControl;

use core::ptr;
use crate::interpreter_action::CallInput;

// ============================ 新增 Super-Instructions ============================

/// Fused instruction: AND 之后对顶部三元素做 Swap1/Pop/Swap2/Swap1 的效果。
///
/// 等价逻辑（以栈顶为索引 0）：
/// 1. pop a, pop b        // 取出两个操作数
/// 2. r = a & b           // 位与运算
/// 3. 读取剩余栈顶 c(0), d(1), e(2)
/// 4. 结果栈应变为 [d, e, r, ...] （将 r 放到第 2 层，其他元素上移）
///
/// 该函数假设在运行前栈深度 ≥ 5，否则会触发 StackUnderflow。
pub(super)fn and_swap1_pop_swap2_swap1<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    // 基础 gas：沿用 AND 指令 (VERYLOW)。多出来的交换成本 EVM 原生为 0。
    gas!(interpreter, 4*gas::VERYLOW+gas::BASE);

    // 1. pop 两个操作数
    popn!([a, b, _c, d, e], interpreter);
    let r = a & b;

    // 3. 以 [r, e, d] 顺序压栈，压栈后栈顶依次是 d, e, r
    push!(interpreter, r);
    push!(interpreter, e);
    push!(interpreter, d);
    interpreter.bytecode.relative_jump(4);
}


/// Fused instruction: SWAP2 SWAP1 POP JUMP
pub(super) fn swap2_swap1_pop_jump<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    // Gas: SWAP2 + SWAP1 + POP + JUMP
    gas!(interpreter, 2*gas::VERYLOW + gas::BASE + gas::MID);

    // Pop two values: `a` will be re-inserted, `_` is discarded.
    popn!([a, _tmp], interpreter);

    // Read current top (will be the jump destination)
    let Some(top) = interpreter.stack.top() else {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    };
    let dest_u256 = *top;
    // Replace top with `a`
    *top = a;

    // Validate jump destination
    let dest = as_usize_or_fail!(interpreter, dest_u256, InstructionResult::InvalidJump);
    if !interpreter.bytecode.is_valid_legacy_jump(dest) {
        interpreter.halt(InstructionResult::InvalidJump);
        return;
    }
    // Perform absolute jump
    interpreter.bytecode.absolute_jump(dest-1);
    // interpreter.bytecode.absolute_jump(dest);
}

/// Fused instruction: SWAP1 POP SWAP2 SWAP1
pub(super) fn swap1_pop_swap2_swap1<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    // Gas: SWAP1 + POP + SWAP2 + SWAP1
    gas!(interpreter, 3*gas::VERYLOW + gas::BASE);

    if !interpreter.stack.exchange(0, 1) {
        interpreter.halt(InstructionResult::StackOverflow);
    }

    // Pop two (top is `a` to be re-inserted)
    popn!([_tmp], interpreter);

    if !interpreter.stack.exchange(0, 2) {
        interpreter.halt(InstructionResult::StackOverflow);
    }

    if !interpreter.stack.exchange(0, 1) {
        interpreter.halt(InstructionResult::StackOverflow);
    }

    // Skip over the remaining 3 bytes of the original sequence
    interpreter.bytecode.relative_jump(3);
}

/// Fused instruction: POP SWAP2 SWAP1 POP
pub(super)fn pop_swap2_swap1_pop<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    // Gas: POP + SWAP2 + SWAP1 + POP
    gas!(interpreter, 2*gas::BASE + 2*gas::VERYLOW);

    // Discard first value, keep `b`
    popn!([ _discard, b, _c ], interpreter);

    let Some(top) = interpreter.stack.top() else {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    };
    let d_val = *top;
    // Replace old `d` with `b`
    *top = b;
    let _ = top;

    // Duplicate `d` value
    push!(interpreter, d_val);
    

    // Skip remaining 3 bytes
    interpreter.bytecode.relative_jump(3);
}

/// Fused instruction: PUSH2 <imm16> JUMP
pub(super)fn push2_jump<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    // Gas: PUSH2 + JUMP
    gas!(interpreter, gas::VERYLOW + gas::MID);

    // Read immediate 2-byte destination (big-endian)
    let imm = interpreter.bytecode.read_slice(2);
    let dest_u256 = U256::from_be_slice(imm);
    let dest = as_usize_or_fail!(interpreter, dest_u256, InstructionResult::InvalidJump);

    if !interpreter.bytecode.is_valid_legacy_jump(dest) {
        interpreter.halt(InstructionResult::InvalidJump);
        return;
    }
    interpreter.bytecode.absolute_jump(dest-1);
}

/// Fused instruction: PUSH2 <imm16> JUMPI
pub(super)fn push2_jumpi<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    // Gas: PUSH2 + JUMPI
    gas!(interpreter, gas::VERYLOW + gas::HIGH);

    // Read immediate destination
    let imm = interpreter.bytecode.read_slice(2);
    let dest_u256 = U256::from_be_slice(imm);
    // Pop condition
    popn!([cond], interpreter);

    if !cond.is_zero() {
        let dest = as_usize_or_fail!(
            interpreter,
            dest_u256,
            InstructionResult::InvalidJump
        );
        if !interpreter.bytecode.is_valid_legacy_jump(dest) {
            interpreter.halt(InstructionResult::InvalidJump);
            return;
        }
        interpreter.bytecode.absolute_jump(dest-1);
    } else {
        // Skip imm16 + NOP (total 3 bytes ahead of current pointer)
        interpreter.bytecode.relative_jump(3);
    }
}

/// Fused instruction: PUSH1 <a> PUSH1 <b>
pub(super)fn push1_push1<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    // Gas: two PUSH1
    gas!(interpreter, 2*gas::VERYLOW);

    let bytes = interpreter.bytecode.read_slice(3);
    let a = U256::from(bytes.get(0).copied().unwrap_or(0u8));
    let b = U256::from(bytes.get(2).copied().unwrap_or(0u8));

    push!(interpreter, a);
    push!(interpreter, b);

    // Skip imm + NOP + imm (3 bytes)
    interpreter.bytecode.relative_jump(3);
}

/// Fused instruction: PUSH1 <imm> ADD
pub(super)fn push1_add<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, 2*gas::VERYLOW);

    let imm = interpreter.bytecode.read_u8() as u64;
    popn!([b], interpreter);
    let res = b + U256::from(imm);
    push!(interpreter, res);

    // Skip imm + NOP (2 bytes)
    interpreter.bytecode.relative_jump(2);
}

/// Fused instruction: PUSH1 <imm> SHL
pub(super)fn push1_shl<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, 2*gas::VERYLOW);

    let shift = interpreter.bytecode.read_u8();
    let Some(val) = interpreter.stack.top() else {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    };

    // `shift` 为 u8，范围已限定在 0..=255，无需再比较。
    *val = *val << (shift as usize);

    interpreter.bytecode.relative_jump(2);
}

/// Fused instruction: PUSH1 <imm> DUP1
pub(super)fn push1_dup1<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, 2*gas::VERYLOW);

    let imm = interpreter.bytecode.read_u8();
    let value = U256::from(imm);
    push!(interpreter, value);
    push!(interpreter, value);

    interpreter.bytecode.relative_jump(2);
}

/// Fused instruction: SWAP1 POP
pub(super)fn swap1_pop<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, gas::VERYLOW + gas::BASE);

    popn!([a], interpreter);
    let Some(b) = interpreter.stack.top() else {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    };
    *b = a;
    interpreter.bytecode.relative_jump(1);
}

/// Fused instruction: POP JUMP
pub(super)fn pop_jump<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, gas::BASE + gas::MID);

    popn!([ _discard, dest_u256 ], interpreter);
    let dest = as_usize_or_fail!(interpreter, dest_u256, InstructionResult::InvalidJump);
    if !interpreter.bytecode.is_valid_legacy_jump(dest) {
        interpreter.halt(InstructionResult::InvalidJump);
        return;
    }
    interpreter.bytecode.absolute_jump(dest-1);
}

/// Fused instruction: POP POP
pub(super)fn pop2<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, 2*gas::BASE);
    popn!([ _a, _b ], interpreter);
    interpreter.bytecode.relative_jump(1);
}

/// Fused instruction: SWAP2 SWAP1
pub(super)fn swap2_swap1<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, 2*gas::VERYLOW);

    if !interpreter.stack.exchange(0, 2) {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    }
    if !interpreter.stack.exchange(0, 1) {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    }
    interpreter.bytecode.relative_jump(1);
}

/// Fused instruction: SWAP2 POP
pub(super)fn swap2_pop<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, gas::VERYLOW + gas::BASE);

    if !interpreter.stack.exchange(0, 2) {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    }
    // Pop the (now) top value
    popn!([ _x ], interpreter);
    interpreter.bytecode.relative_jump(1);
}

/// Fused instruction: DUP2 LT
pub(super)fn dup2_lt<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, 2*gas::VERYLOW);

    // Duplicate 2nd item to top then perform LT
    if !interpreter.stack.dup(2) {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    }

    // Pop the two operands
    popn!([a, b], interpreter);
    let result = if b < a { U256::ONE } else { U256::ZERO };
    push!(interpreter, result);

    interpreter.bytecode.relative_jump(1);
}

/// Fused instruction: ISZERO PUSH2 .. JUMPI  => JUMPIFZERO
pub(super)fn jump_if_zero<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    // Approximated gas: ISZERO (VERYLOW) + JUMPI (HIGH)
    gas!(interpreter, gas::VERYLOW + gas::HIGH);

    // Pop condition value
    popn!([value], interpreter);

    if value.is_zero() {
        // Immediate destination is 2 bytes located 2 bytes ahead (skip NOP + imm16)
        let dest_u16 = interpreter.bytecode.read_offset_u16(2);
        let dest = dest_u16 as usize;
        if !interpreter.bytecode.is_valid_legacy_jump(dest) {
            interpreter.halt(InstructionResult::InvalidJump);
            return;
        }
        interpreter.bytecode.absolute_jump(dest-1);
    } else {
        // Skip the rest (NOP + imm16 + NOP) => 4 bytes
        interpreter.bytecode.relative_jump(4);
    }
}

/// Super NOP instruction (SNOP)
pub(super)fn snop<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    // Zero-cost, zero-effect.
    gas!(interpreter, gas::ZERO);
    // Nothing else to do.
}


/// Fused instruction: ISZERO PUSH2 <imm16>
pub(super)fn iszero_push2<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, 2 * gas::VERYLOW);

    // Mutate top of stack
    let Some(x) = interpreter.stack.top() else {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    };
    if x.is_zero() {
        *x = U256::ONE;
    } else {
        *x = U256::ZERO;
    }
    let _ = x;

    // Current PC is at NOP (byte after fused opcode)
    // Immediate bytes are located at offset 1 and 2
    let imm16 = interpreter.bytecode.read_offset_u16(1);
    push!(interpreter, U256::from(imm16));

    // Skip NOP + imm16 (3 bytes total)
    interpreter.bytecode.relative_jump(3);
}


/// Fused instruction: DUP2 MSTORE PUSH1 _ ADD
pub(super)fn dup2_mstore_push1_add<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    // Gas: MSTORE (VERYLOW) + PUSH1 + ADD + small overhead
    gas!(interpreter, 3*gas::VERYLOW + gas::BASE);

    // Pop value to store
    popn!([val], interpreter);

    // Obtain offset (now at stack top)
    let Some(offset_ref) = interpreter.stack.top() else {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    };
    let offset_usize = as_usize_or_fail!(interpreter, *offset_ref);
    // Resize memory and store 32-byte word
    resize_memory!(interpreter, offset_usize, 32);
    interpreter
        .memory
        .set(offset_usize, &val.to_be_bytes::<32>());

    // Read immediate byte (located +2 from current ptr: NOP + imm)
    let imm = interpreter.bytecode.read_slice(3)[2];
    let res = *offset_ref + U256::from(imm);
    *offset_ref = res;

    // Skip remaining bytes (NOP, imm, NOP) => 4 bytes
    interpreter.bytecode.relative_jump(4);
}

/// Fused instruction: DUP1 PUSH4 imm EQ PUSH2 imm2
pub(super)fn dup1_push4_eq_push2<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, 4*gas::VERYLOW + gas::BASE);

    // Duplicate top
    if !interpreter.stack.dup(1) {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    }

    // Read 4-byte constant (offsets 2-5 from current ptr)
    let bytes4 = {
        let slice = interpreter.bytecode.read_slice(6);
        [slice[2], slice[3], slice[4], slice[5]]
    };
    let const_val = U256::from_be_slice(&bytes4);
    push!(interpreter, const_val);

    // Equality check
    popn!([a, x], interpreter); // a = const_val, x = duplicated original
    let eq = if a == x { U256::ONE } else { U256::ZERO };
    push!(interpreter, eq);

    // Read 2-byte immediate for PUSH2 (offset 8-9 from current ptr)
    let dest_u16 = interpreter.bytecode.read_offset_u16(7);
    push!(interpreter, U256::from(dest_u16));

    // Skip remaining bytes to end of fused sequence (total 10 ⇒ skip 9)
    interpreter.bytecode.relative_jump(9);
}

/// Fused instruction: PUSH1 CALLDATALOAD PUSH1 SHR DUP1 PUSH4 GT PUSH2
pub(super)fn push1_calldataload_push1_shr_dup1_push4_gt_push2<
    WIRE: InterpreterTypes,
    H: ?Sized,
>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    // Rough gas: PUSH1+CALLDATALOAD+PUSH1+SHR+DUP1+PUSH4+GT+PUSH2
    gas!(
        interpreter,
        4 * gas::VERYLOW + gas::MID + gas::HIGH + gas::BASE
    );

    // Read immediate offset (1 byte right after opcode)
    let bytes = interpreter.bytecode.read_slice(16);
    if bytes.len() < 16 {
        interpreter.halt(InstructionResult::InvalidOperandOOG);
        return;
    }
    let offset_byte = bytes[0] as usize;

    // Load 32 bytes from calldata at offset
    let mut word = B256::ZERO;
    let input = interpreter.input.input();
    let input_len = input.len();
    if offset_byte < input_len {
        let count = 32.min(input_len - offset_byte);
        match input {
            CallInput::Bytes(bytes) => unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr().add(offset_byte), word.as_mut_ptr(), count);
            },
            CallInput::SharedBuffer(range) => {
                let slice = interpreter.memory.global_slice(range.clone());
                unsafe {
                    ptr::copy_nonoverlapping(slice.as_ptr().add(offset_byte), word.as_mut_ptr(), count);
                }
            }
        }
    }
    let mut x = U256::from_be_bytes(word.0);

    // Read shift immediate (byte 4 in slice: index 3 is PUSH1 opcode (NOP), index 4 is imm)
    let shift_byte = bytes[4] as usize;
    // 立即数来自字节，天然 <256，可直接右移
    x >>= shift_byte;

    // Push x
    push!(interpreter, x);
    // DUP1
    if !interpreter.stack.dup(1) {
        interpreter.halt(InstructionResult::StackOverflow);
        return;
    }

    // Constant 4-byte big-endian located starting at index 8..12
    let const_val = U256::from_be_slice(&bytes[8..12]);

    push!(interpreter, const_val);

    // GT: compare const_val (a) and copy of x (p)
    popn_top!([_a], p_ref, interpreter);
    if const_val > *p_ref {
        *p_ref = U256::ONE;
    } else {
        *p_ref = U256::ZERO;
    }

    // PUSH2 dest (index 14,15)
    let dest_u16 = ((bytes[14] as u16) << 8) | bytes[15] as u16;
    push!(interpreter, U256::from(dest_u16));

    // Skip remaining 15 bytes (pattern length 16)
    interpreter.bytecode.relative_jump(15);
}

/// Fused instruction: PUSH1 PUSH1 PUSH1 SHL SUB
pub(super)fn push1_push1_push1_shl_sub<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, 2 * gas::VERYLOW + gas::BASE);

    // Read the three immediates
    let bytes = interpreter.bytecode.read_slice(8);
    if bytes.len() < 8 {
        interpreter.halt(InstructionResult::InvalidOperandOOG);
        return;
    }
    let imm1 = U256::from(bytes[0]); // first PUSH1 immediate
    let imm2 = U256::from(bytes[2]); // second
    let imm3 = bytes[4] as usize; // third is shift amount

    // Compute result: (imm2 << imm3) - imm1 (imm3 最大 255，安全)
    let mut res = imm2 << imm3;
    res = res.wrapping_sub(imm1);

    push!(interpreter, res);

    // Skip remaining 7 bytes
    interpreter.bytecode.relative_jump(7);
}

/// Fused instruction: SWAP1 PUSH1 DUP1 NOT SWAP2 ADD AND DUP2 ADD SWAP1 DUP2 LT
pub(super)fn swap1_push1_dup1_not_swap2_add_and_dup2_add_swap1_dup2_lt<
    WIRE: InterpreterTypes,
    H: ?Sized,
>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, 10 * gas::VERYLOW);

    // 1. SWAP1
    if !interpreter.stack.exchange(0, 1) {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    }

    // 2. PUSH1 immediate (byte index 2)
    let imm = interpreter.bytecode.read_slice(3)[2];
    push!(interpreter, U256::from(imm));

    // 3. DUP1
    if !interpreter.stack.dup(1) {
        interpreter.halt(InstructionResult::StackOverflow);
        return;
    }

    // 4. NOT on top
    let Some(top) = interpreter.stack.top() else {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    };
    *top = !*top;
    let _ = top;

    // 5. SWAP2
    if !interpreter.stack.exchange(0, 2) {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    }

    // 6. ADD
    popn!([a_val, b_val], interpreter);
    let sum = a_val + b_val;
    push!(interpreter, sum);

    // 7. AND
    popn!([c_val, d_val], interpreter);
    let anded = c_val & d_val;
    push!(interpreter, anded);

    // 8. DUP2
    if !interpreter.stack.dup(2) {
        interpreter.halt(InstructionResult::StackOverflow);
        return;
    }

    // 9. ADD
    popn!([e_val, f_val], interpreter);
    let add2 = e_val + f_val;
    push!(interpreter, add2);

    // 10. SWAP1
    if !interpreter.stack.exchange(0, 1) {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    }

    // 11. DUP2
    if !interpreter.stack.dup(2) {
        interpreter.halt(InstructionResult::StackOverflow);
        return;
    }

    // 12. LT
    popn_top!([g_val], h_ref, interpreter);
    if g_val < *h_ref {
        *h_ref = U256::ONE;
    } else {
        *h_ref = U256::ZERO;
    }

    // Skip remaining 12 bytes (pattern length 13)
    interpreter.bytecode.relative_jump(12);
}

/// Fused instruction: AND DUP2 ADD SWAP1 DUP2 LT
pub(super)fn and_dup2_add_swap1_dup2_lt<WIRE: InterpreterTypes, H: ?Sized>(
    interpreter: &mut Interpreter<WIRE>,
    _host: &mut H,
) {
    gas!(interpreter, 5 * gas::VERYLOW);

    // Step 1: AND (pop x, y; push y&x)
    popn!([x_val], interpreter);
    popn_top!([], top_ref, interpreter);
    let mut y = *top_ref & x_val;
    *top_ref = y; // write back
    let _ =top_ref; // release borrow

    // Step 2: DUP2
    if !interpreter.stack.dup(2) {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    }

    // Step 3: ADD (pop z, add to y)
    popn!([z_val], interpreter);
    popn_top!([], y_mut, interpreter);
    y = *y_mut + z_val;
    *y_mut = y;
    let _ = y_mut;

    // SWAP1
    if !interpreter.stack.exchange(0, 1) {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    }

    // DUP2
    if !interpreter.stack.dup(2) {
        interpreter
            .control
            .set_instruction_result(InstructionResult::StackUnderflow);
        return;
    }

    // LT
    popn_top!([a_val], b_ref, interpreter);
    if a_val < *b_ref {
        *b_ref = U256::ONE;
    } else {
        *b_ref = U256::ZERO;
    }

    // Skip remaining 5 bytes (pattern length 6)
    interpreter.bytecode.relative_jump(5);
}



