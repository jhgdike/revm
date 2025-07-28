use crate::{
    gas,
    interpreter_types::{InterpreterTypes, StackTr, Jumps, Immediates, MemoryTr, InputsTr},
    InstructionContext,
};
use primitives::{U256, B256};
use crate::InstructionResult;

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
    context: InstructionContext<'_, H, WIRE>,
) {
    // 基础 gas：沿用 AND 指令 (VERYLOW)。多出来的交换成本 EVM 原生为 0。
    gas!(context.interpreter, 4*gas::VERYLOW+gas::BASE);

    // 1. pop 两个操作数
    popn!([a, b], context.interpreter);
    let r = a & b;
    backn!([c, d, e], context.interpreter);
    *c = *d;
    *d = *e;
    *e = r;

    context.interpreter.bytecode.relative_jump(4);
}


/// Fused instruction: SWAP2 SWAP1 POP JUMP
pub(super) fn swap2_swap1_pop_jump<WIRE: InterpreterTypes, H: ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    // Gas: SWAP2 + SWAP1 + POP + JUMP
    gas!(context.interpreter, 2*gas::VERYLOW + gas::BASE + gas::MID);

    // Pop two values: `a` will be re-inserted, `_` is discarded.
    popn!([a, _tmp], context.interpreter);

    // Read current top (will be the jump destination)
    backn!([top], context.interpreter);
    let dest_u256 = *top;
    // Replace top with `a`
    *top = a;

    // Validate jump destination
    let dest = as_usize_or_fail!(context.interpreter, dest_u256, InstructionResult::InvalidJump);
    if !context.interpreter.bytecode.is_valid_legacy_jump(dest) {
        context.interpreter.halt(InstructionResult::InvalidJump);
        return;
    }
    // Perform absolute jump
    context.interpreter.bytecode.absolute_jump(dest-1);
    // context.interpreter.bytecode.absolute_jump(dest);
}

/// Fused instruction: SWAP1 POP SWAP2 SWAP1
pub(super) fn swap1_pop_swap2_swap1<WIRE: InterpreterTypes, H: ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    // Gas: SWAP1 + POP + SWAP2 + SWAP1
    gas!(context.interpreter, 3*gas::VERYLOW + gas::BASE);

    // if !context.interpreter.stack.exchange(0, 1) {
    //     context.interpreter.halt(InstructionResult::StackOverflow);
    // }

    // // Pop two (top is `a` to be re-inserted)
    // popn!([_tmp], context.interpreter);

    // if !context.interpreter.stack.exchange(0, 2) {
    //     context.interpreter.halt(InstructionResult::StackOverflow);
    // }

    // if !context.interpreter.stack.exchange(0, 1) {
    //     context.interpreter.halt(InstructionResult::StackOverflow);
    // }
    popn!([a], context.interpreter);
    backn!([b, c, d], context.interpreter);
    *b = *c;
    *c = *d;
    *d = a;

    // Skip over the remaining 3 bytes of the original sequence
    context.interpreter.bytecode.relative_jump(3);
}

/// Fused instruction: POP SWAP2 SWAP1 POP
pub(super)fn pop_swap2_swap1_pop<WIRE: InterpreterTypes, H: ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    // Gas: POP + SWAP2 + SWAP1 + POP
    gas!(context.interpreter, 2*gas::BASE + 2*gas::VERYLOW);

    // Discard first value, keep `b`
    popn!([ _discard, b ], context.interpreter);
    backn!([c, d], context.interpreter);
    *c = *d;
    *d = b;

    // Skip remaining 3 bytes
    context.interpreter.bytecode.relative_jump(3);
}

/// Fused instruction: PUSH2 <imm16> JUMP
pub(super)fn push2_jump<WIRE: InterpreterTypes, H: ?Sized>(context: InstructionContext<'_, H, WIRE>) {
    // Gas: PUSH2 + JUMP
    gas!(context.interpreter, gas::VERYLOW + gas::MID);

    // Read immediate 2-byte destination (big-endian)
    let imm = context.interpreter.bytecode.read_slice(2);
    let dest = as_usize_or_fail!(context.interpreter, U256::from_be_slice(imm), InstructionResult::InvalidJump);

    if !context.interpreter.bytecode.is_valid_legacy_jump(dest) {
        context.interpreter.halt(InstructionResult::InvalidJump);
        return;
    }
    context.interpreter.bytecode.absolute_jump(dest-1);
}

/// Fused instruction: PUSH2 <imm16> JUMPI
pub(super)fn push2_jumpi<WIRE: InterpreterTypes, H: ?Sized>(context: InstructionContext<'_, H, WIRE>) {
    // Gas: PUSH2 + JUMPI
    gas!(context.interpreter, gas::VERYLOW + gas::HIGH);

    // Read immediate destination
    let imm = context.interpreter.bytecode.read_slice(2);
    // Pop condition
    popn!([cond], context.interpreter);

    if !cond.is_zero() {
        let dest = as_usize_or_fail!(
            context.interpreter,
            U256::from_be_slice(imm),
            InstructionResult::InvalidJump
        );
        if !context.interpreter.bytecode.is_valid_legacy_jump(dest) {
            context.interpreter.halt(InstructionResult::InvalidJump);
            return;
        }
        context.interpreter.bytecode.absolute_jump(dest-1);
    } else {
        // Skip imm16 + NOP (total 3 bytes ahead of current pointer)
        context.interpreter.bytecode.relative_jump(3);
    }
}

/// Fused instruction: PUSH1 <a> PUSH1 <b>
pub(super)fn push1_push1<WIRE: InterpreterTypes, H: ?Sized>(context: InstructionContext<'_, H, WIRE>) {
    // Gas: two PUSH1
    gas!(context.interpreter, 2*gas::VERYLOW);

    let bytes = context.interpreter.bytecode.read_slice(3);
    let a = U256::from(bytes.get(0).copied().unwrap_or(0u8));
    let b = U256::from(bytes.get(2).copied().unwrap_or(0u8));

    push!(context.interpreter, a);
    push!(context.interpreter, b);

    // Skip imm + NOP + imm (3 bytes)
    context.interpreter.bytecode.relative_jump(3);
}

/// Fused instruction: PUSH1 <imm> ADD
pub(super)fn push1_add<WIRE: InterpreterTypes, H: ?Sized>(context: InstructionContext<'_, H, WIRE>) {
    gas!(context.interpreter, 2*gas::VERYLOW);

    let imm = context.interpreter.bytecode.read_u8() as u64;
    backn!([b], context.interpreter);
    *b = *b + U256::from(imm);
    // push!(context.interpreter, res);

    // Skip imm + NOP (2 bytes)
    context.interpreter.bytecode.relative_jump(2);
}

/// Fused instruction: PUSH1 <imm> SHL
pub(super)fn push1_shl<WIRE: InterpreterTypes, H: ?Sized>(context: InstructionContext<'_, H, WIRE>) {
    gas!(context.interpreter, 2*gas::VERYLOW);

    let shift = context.interpreter.bytecode.read_u8();
    backn!([val], context.interpreter);

    // `shift` 为 u8，范围已限定在 0..=255，无需再比较。
    *val = *val << (shift as usize);

    context.interpreter.bytecode.relative_jump(2);
}

/// Fused instruction: PUSH1 <imm> DUP1
pub(super)fn push1_dup1<WIRE: InterpreterTypes, H: ?Sized>(context: InstructionContext<'_, H, WIRE>) {
    gas!(context.interpreter, 2*gas::VERYLOW);

    let imm = context.interpreter.bytecode.read_u8();
    let value = U256::from(imm);
    push!(context.interpreter, value);
    push!(context.interpreter, value);

    context.interpreter.bytecode.relative_jump(2);
}

/// Fused instruction: SWAP1 POP
pub(super)fn swap1_pop<WIRE: InterpreterTypes, H: ?Sized>(context: InstructionContext<'_, H, WIRE>) {
    gas!(context.interpreter, gas::VERYLOW + gas::BASE);

    popn!([a], context.interpreter);
    let Some(b) = context.interpreter.stack.top() else {
        context.interpreter.halt(InstructionResult::StackUnderflow);
        return;
    };
    *b = a;
    context.interpreter.bytecode.relative_jump(1);
}

/// Fused instruction: POP JUMP
pub(super)fn pop_jump<WIRE: InterpreterTypes, H: ?Sized>(context: InstructionContext<'_, H, WIRE>) {
    gas!(context.interpreter, gas::BASE + gas::MID);

    popn!([ _discard, dest_u256 ], context.interpreter);
    let dest = as_usize_or_fail!(context.interpreter, dest_u256, InstructionResult::InvalidJump);
    if !context.interpreter.bytecode.is_valid_legacy_jump(dest) {
        context.interpreter.halt(InstructionResult::InvalidJump);
        return;
    }
    context.interpreter.bytecode.absolute_jump(dest-1);
}

/// Fused instruction: POP POP
pub(super)fn pop2<WIRE: InterpreterTypes, H: ?Sized>(context: InstructionContext<'_, H, WIRE>) {
    gas!(context.interpreter, 2*gas::BASE);
    popn!([ _a, _b ], context.interpreter);
    context.interpreter.bytecode.relative_jump(1);
}

/// Fused instruction: SWAP2 SWAP1
pub(super)fn swap2_swap1<WIRE: InterpreterTypes, H: ?Sized>(context: InstructionContext<'_, H, WIRE>) {
    gas!(context.interpreter, 2*gas::VERYLOW);
    backn!([a, b, c], context.interpreter);
    let tmp = *a;
    *a = *b;
    *b = *c;
    *c = tmp;

    // if !context.interpreter.stack.exchange(0, 2) {
    //     context.interpreter.halt(InstructionResult::StackUnderflow);
    //     return;
    // }
    // if !context.interpreter.stack.exchange(0, 1) {
    //     context.interpreter.halt(InstructionResult::StackUnderflow);
    //     return;
    // }
    context.interpreter.bytecode.relative_jump(1);
}

/// Fused instruction: SWAP2 POP
pub(super)fn swap2_pop<WIRE: InterpreterTypes, H: ?Sized>(context: InstructionContext<'_, H, WIRE>) {
    gas!(context.interpreter, gas::VERYLOW + gas::BASE);

    // if !context.interpreter.stack.exchange(0, 2) {
    //     context.interpreter.halt(InstructionResult::StackUnderflow);
    //     return;
    // }
    backn!([a, _b, c], context.interpreter);
    *c = *a;
    // Pop the (now) top value
    popn!([ _x ], context.interpreter);
    context.interpreter.bytecode.relative_jump(1);
}

/// Fused instruction: DUP2 LT
pub(super)fn dup2_lt<WIRE: InterpreterTypes, H: ?Sized>(context: InstructionContext<'_, H, WIRE>) {
    gas!(context.interpreter, 2*gas::VERYLOW);

    // Duplicate 2nd item to top then perform LT
    // if !context.interpreter.stack.dup(2) {
    //     context.interpreter.halt(InstructionResult::StackUnderflow);
    //     return;
    // }

    // Pop the two operands
    // popn!([a, b], context.interpreter);
    backn!([a, b], context.interpreter);
    *a = if *b < *a { U256::ONE } else { U256::ZERO };
    // push!(context.interpreter, result);

    context.interpreter.bytecode.relative_jump(1);
}

/// Fused instruction: ISZERO PUSH2 .. JUMPI  => JUMPIFZERO
pub(super)fn jump_if_zero<WIRE: InterpreterTypes, H: ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    // Approximated gas: ISZERO (VERYLOW) + JUMPI (HIGH)
    gas!(context.interpreter, 2*gas::VERYLOW + gas::HIGH);

    // Pop condition value
    popn!([value], context.interpreter);

    if value.is_zero() {
        // Immediate destination is 2 bytes located 2 bytes ahead (skip NOP + imm16)
        let dest = context.interpreter.bytecode.read_offset_u16(2) as usize;
        // let dest = dest_u16 as usize;
        if !context.interpreter.bytecode.is_valid_legacy_jump(dest) {
            context.interpreter.halt(InstructionResult::InvalidJump);
            return;
        }
        context.interpreter.bytecode.absolute_jump(dest-1);
    } else {
        // Skip the rest (NOP + imm16 + NOP) => 4 bytes
        context.interpreter.bytecode.relative_jump(4);
    }
}

/// Super NOP instruction (SNOP)
pub(super)fn snop<WIRE: InterpreterTypes, H: ?Sized>(_context: InstructionContext<'_, H, WIRE>) {
    // Zero-cost, zero-effect.
    // gas!(context.interpreter, gas::ZERO);
    // Nothing else to do.
}


/// Fused instruction: ISZERO PUSH2 <imm16>
pub(super)fn iszero_push2<WIRE: InterpreterTypes, H: ?Sized>(context: InstructionContext<'_, H, WIRE>) {
    gas!(context.interpreter, 2 * gas::VERYLOW);

    // Mutate top of stack
    let Some(x) = context.interpreter.stack.top() else {
        context.interpreter.halt(InstructionResult::StackUnderflow);
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
    let imm16 = context.interpreter.bytecode.read_offset_u16(1);
    push!(context.interpreter, U256::from(imm16));

    // Skip NOP + imm16 (3 bytes total)
    context.interpreter.bytecode.relative_jump(3);
}


/// Fused instruction: DUP2 MSTORE PUSH1 _ ADD
pub(super)fn dup2_mstore_push1_add<WIRE: InterpreterTypes, H: ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    // Gas: MSTORE (VERYLOW) + PUSH1 + ADD + small overhead
    gas!(context.interpreter, 4*gas::VERYLOW);

    // Pop value to store
    popn!([val], context.interpreter);

    // Obtain offset (now at stack top)
    let Some(offset_ref) = context.interpreter.stack.top() else {
        context.interpreter.halt(InstructionResult::StackUnderflow);
        return;
    };
    let offset_usize = as_usize_or_fail!(context.interpreter, *offset_ref);
    // Resize memory and store 32-byte word
    resize_memory!(context.interpreter, offset_usize, 32);
    context
        .interpreter
        .memory
        .set(offset_usize, &val.to_be_bytes::<32>());

    // Read immediate byte (located +2 from current ptr: NOP + imm)
    let imm = context.interpreter.bytecode.read_slice(3)[2];
    *offset_ref = *offset_ref + U256::from(imm);
    // *offset_ref = res;

    // Skip remaining bytes (NOP, imm, NOP) => 4 bytes
    context.interpreter.bytecode.relative_jump(4);
}

/// Fused instruction: DUP1 PUSH4 imm EQ PUSH2 imm2
pub(super)fn dup1_push4_eq_push2<WIRE: InterpreterTypes, H: ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    // gas!(context.interpreter, 4*gas::VERYLOW + gas::BASE);
    gas!(context.interpreter, 4*gas::VERYLOW);

    // Duplicate top
    if !context.interpreter.stack.dup(1) {
        context.interpreter.halt(InstructionResult::StackUnderflow);
        return;
    }

    // Read 4-byte constant (offsets 2-5 from current ptr)
    let bytes4 = {
        let slice = context.interpreter.bytecode.read_slice(6);
        [slice[2], slice[3], slice[4], slice[5]]
    };
    let const_val = U256::from_be_slice(&bytes4);
    backn!([x], context.interpreter);
    *x = if const_val == *x {U256::ONE} else { U256::ZERO }; 
    // push!(context.interpreter, const_val);

    // // Equality check
    // popn!([a, x], context.interpreter); // a = const_val, x = duplicated original
    // let eq = if a == x { U256::ONE } else { U256::ZERO };
    // push!(context.interpreter, eq);

    // Read 2-byte immediate for PUSH2 (offset 8-9 from current ptr)
    let dest_u16 = context.interpreter.bytecode.read_offset_u16(7);
    push!(context.interpreter, U256::from(dest_u16));

    // Skip remaining bytes to end of fused sequence (total 10 ⇒ skip 9)
    context.interpreter.bytecode.relative_jump(9);
}

#[inline]
fn hi31_is_zero(word: &B256) -> bool {
    // 把前 24 字节当成 3 × u64 读取，再取第 25~31 字节组成的 u64
    // => 只要 OR 后结果为 0，即全部为 0
    let p = word.as_ptr() as *const u64;
    // SAFETY: B256 恰好 32 字节，对齐到 u8；逐 8 字节读取合法
    unsafe {
        let v0 = *p;               // byte  0‥7
        let v1 = *p.add(1);        // byte  8‥15
        let v2 = *p.add(2);        // byte 16‥23
        let last = *p.add(3) & 0xffff_ffff_ffff_ff00u64; // 去掉最低 1 byte
        (v0 | v1 | v2 | last) == 0
    }
}

/// Fused instruction: PUSH1 CALLDATALOAD PUSH1 SHR DUP1 PUSH4 GT PUSH2
pub(super)fn push1_calldataload_push1_shr_dup1_push4_gt_push2<
    WIRE: InterpreterTypes,
    H: ?Sized,
>(context: InstructionContext<'_, H, WIRE>) {
    // Rough gas: PUSH1+CALLDATALOAD+PUSH1+SHR+DUP1+PUSH4+GT+PUSH2
    // gas!(
    //     context.interpreter,
    //     4 * gas::VERYLOW + gas::MID + gas::HIGH + gas::BASE
    // );
    gas!(context.interpreter, 8*gas::VERYLOW);

    // Read immediate offset (1 byte right after opcode)
    let bytes = context.interpreter.bytecode.read_slice(15);
    if bytes.len() < 15 {
        context.interpreter.halt(InstructionResult::InvalidOperandOOG);
        return;
    }
    let offset_byte = bytes[0] as usize;

    // Load 32 bytes from calldata at offset
    let mut word = B256::ZERO;
    let input = context.interpreter.input.input();
    let input_len = input.len();
    if offset_byte < input_len {
        let count = 32.min(input_len - offset_byte);
        match input {
            CallInput::Bytes(bytes) => unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr().add(offset_byte), word.as_mut_ptr(), count);
            },
            CallInput::SharedBuffer(range) => {
                let slice = context.interpreter.memory.global_slice(range.clone());
                unsafe {
                    ptr::copy_nonoverlapping(slice.as_ptr().add(offset_byte), word.as_mut_ptr(), count);
                }
            }
        }
    }
    // let mut x = U256::from_be_bytes();

    // Read shift immediate (byte 4 in slice: index 3 is PUSH1 opcode (NOP), index 4 is imm)
    let shift_byte = bytes[3] as usize;
    let mut x = U256::ZERO;
    // 立即数来自字节，天然 <256，可直接右移
    if hi31_is_zero(&word) {
        x = U256::from(word.0[31]) >> shift_byte;
    }

    // Push x
    push!(context.interpreter, x);
    push!(context.interpreter, x);
    // DUP1
    // if !context.interpreter.stack.dup(1) {
    //     context.interpreter.halt(InstructionResult::StackOverflow);
    //     return;
    // }

    // Constant 4-byte big-endian located starting at index 8..12
    let const_val = U256::from_be_slice(&bytes[7..11]);

    // push!(context.interpreter, const_val);
    backn!([p_ref], context.interpreter);

    // GT: compare const_val (a) and copy of x (p)
    // popn_top!([_a], p_ref, context.interpreter);
    if const_val > *p_ref {
        *p_ref = U256::ONE;
    } else {
        *p_ref = U256::ZERO;
    }

    // PUSH2 dest (index 14,15)
    let dest_u16 = ((bytes[13] as u16) << 8) | bytes[14] as u16;
    push!(context.interpreter, U256::from(dest_u16));

    // Skip remaining 15 bytes (pattern length 16)
    context.interpreter.bytecode.relative_jump(15);
}

/// Fused instruction: PUSH1 PUSH1 PUSH1 SHL SUB
pub(super)fn push1_push1_push1_shl_sub<WIRE: InterpreterTypes, H: ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    // gas!(context.interpreter, 2 * gas::VERYLOW + gas::BASE);
    gas!(context.interpreter, 5*gas::VERYLOW);

    // Read the three immediates
    let bytes = context.interpreter.bytecode.read_slice(7);
    if bytes.len() < 7 {
        context.interpreter.halt(InstructionResult::InvalidOperandOOG);
        return;
    }
    let imm1 = U256::from(bytes[0]); // first PUSH1 immediate
    let imm2 = U256::from(bytes[2]); // second
    let imm3 = bytes[4] as usize; // third is shift amount

    // Compute result: (imm2 << imm3) - imm1 (imm3 最大 255，安全)
    let mut res = imm2 << imm3;
    res = res.wrapping_sub(imm1);

    push!(context.interpreter, res);

    // Skip remaining 7 bytes
    context.interpreter.bytecode.relative_jump(7);
}

/// Fused instruction: SWAP1 PUSH1 DUP1 NOT SWAP2 ADD AND DUP2 ADD SWAP1 DUP2 LT
pub(super)fn swap1_push1_dup1_not_swap2_add_and_dup2_add_swap1_dup2_lt<
    WIRE: InterpreterTypes,
    H: ?Sized,
>(context: InstructionContext<'_, H, WIRE>) {
    // gas!(context.interpreter, 10 * gas::VERYLOW);
    gas!(context.interpreter, 12*gas::VERYLOW);

    // 1. SWAP1
    // if !context.interpreter.stack.exchange(0, 1) {
    //     context.interpreter.halt(InstructionResult::StackUnderflow);
    //     return;
    // }

    backn!([a,b], context.interpreter);

    // 2. PUSH1 immediate (byte index 2)
    let imm = context.interpreter.bytecode.read_slice(2)[1];
    // push!(context.interpreter, U256::from(imm));
    *b = *a+*b+U256::from(imm)+U256::from(!imm);

    if *b < *a {
        *a = U256::ONE;
    } else {
        *a = U256::ZERO;
    }

    // Skip remaining 12 bytes (pattern length 13)
    context.interpreter.bytecode.relative_jump(12);
}

/// Fused instruction: AND DUP2 ADD SWAP1 DUP2 LT
pub(super)fn and_dup2_add_swap1_dup2_lt<WIRE: InterpreterTypes, H: ?Sized>(
    context: InstructionContext<'_, H, WIRE>,
) {
    // gas!(context.interpreter, 5 * gas::VERYLOW);
    gas!(context.interpreter, 6*gas::VERYLOW);

    // Step 1: AND (pop x, y; push y&x)
    popn!([a], context.interpreter);
    backn!([b, c], context.interpreter);
    let tmp = *c;
    *c = a+*b+*c;
    if *c < tmp {
        *b = U256::ONE;
    } else {
        *b = U256::ZERO;
    }

    // Skip remaining 5 bytes (pattern length 6)
    context.interpreter.bytecode.relative_jump(5);
}



