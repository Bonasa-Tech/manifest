# sBPF Assembly: Remove Fix for Red-Black Tree
#
# Function: void remove_fix_asm(u8* data, u32 current, u32 parent,
#                               u32* root_ptr, u32* out_pair)
# Args: r1=data, r2=current, r3=parent, r4=root_ptr, r5=out_pair
# Output: out_pair[0] = next current, out_pair[1] = next parent

.globl remove_fix_asm
remove_fix_asm:
    stxdw [r10 - 8], r6
    stxdw [r10 - 16], r7
    stxdw [r10 - 24], r8
    stxdw [r10 - 32], r9

    stxdw [r10 - 40], r1
    stxdw [r10 - 48], r2
    stxdw [r10 - 56], r3
    stxdw [r10 - 64], r4
    stxdw [r10 - 72], r5
    lddw r6, 4294967295

    ldxw r7, [r4 + 0]
    jne r7, r2, .Lremove_not_at_root
    ja .Lremove_output_nil_nil

.Lremove_not_at_root:
    mov64 r4, r1
    add64 r4, r3
    ldxw r7, [r4 + 0]
    jne r7, r2, .Lremove_current_is_not_left
    ldxw r8, [r4 + 4]
    mov64 r9, 0
    ja .Lremove_have_sibling
.Lremove_current_is_not_left:
    mov64 r8, r7
    mov64 r9, 1
.Lremove_have_sibling:
    stxdw [r10 - 80], r8
    stxdw [r10 - 120], r9

    ldxb r7, [r4 + 12]
    stxdw [r10 - 88], r7
    mov64 r7, 0
    jeq r8, r6, .Lremove_have_sibling_color
    mov64 r4, r1
    add64 r4, r8
    ldxb r7, [r4 + 12]
.Lremove_have_sibling_color:
    stxdw [r10 - 96], r7
    mov64 r4, 0
    jne r7, r4, .Lremove_sibling_red
    lddw r7, 4294967295
    lddw r8, 4294967295
    mov64 r4, 0
    mov64 r5, 0

    ldxdw r9, [r10 - 80]
    jeq r9, r6, .Lremove_sibling_black_no_red_children
    ldxdw r1, [r10 - 40]
    mov64 r2, r1
    add64 r2, r9
    ldxw r7, [r2 + 0]
    ldxw r8, [r2 + 4]
    stxdw [r10 - 104], r7
    stxdw [r10 - 112], r8
    jeq r7, r6, .Lremove_check_right_child
    mov64 r3, r1
    add64 r3, r7
    ldxb r3, [r3 + 12]
    mov64 r5, 1
    jne r3, r5, .Lremove_check_right_child
    mov64 r4, 1

.Lremove_check_right_child:
    jeq r8, r6, .Lremove_have_child_red_flags
    ldxdw r1, [r10 - 40]
    mov64 r3, r1
    add64 r3, r8
    ldxb r3, [r3 + 12]
    mov64 r5, 1
    jne r3, r5, .Lremove_have_child_red_flags
    mov64 r5, 1

.Lremove_have_child_red_flags:
    stxdw [r10 - 128], r4
    stxdw [r10 - 136], r5
    jne r4, 0, .Lremove_sibling_black_has_red_child
    jne r5, 0, .Lremove_sibling_black_has_red_child
    ja .Lremove_sibling_black_no_red_children

.Lremove_sibling_black_has_red_child:
    ldxdw r4, [r10 - 128]
    ldxdw r5, [r10 - 120]
    jeq r4, 0, .Lremove_try_left_right
    jeq r5, 0, .Lremove_try_left_right

.Lremove_case_left_left:
    ldxdw r1, [r10 - 40]
    ldxdw r7, [r10 - 104]
    mov64 r2, r1
    add64 r2, r7
    mov64 r3, 0
    stxb [r2 + 12], r3
    ldxdw r7, [r10 - 56]
    mov64 r2, r1
    add64 r2, r7
    ldxdw r3, [r10 - 96]
    stxb [r2 + 12], r3
    ldxdw r7, [r10 - 80]
    mov64 r2, r1
    add64 r2, r7
    ldxdw r3, [r10 - 88]
    stxb [r2 + 12], r3
    ldxdw r2, [r10 - 56]
    ldxdw r3, [r10 - 64]
    call rotate_right_asm
    ja .Lremove_output_nil_nil

.Lremove_try_left_right:
    ldxdw r4, [r10 - 136]
    ldxdw r5, [r10 - 120]
    jeq r4, 0, .Lremove_try_right_right
    jeq r5, 0, .Lremove_try_right_right

.Lremove_case_left_right:
    ldxdw r1, [r10 - 40]
    ldxdw r7, [r10 - 112]
    mov64 r2, r1
    add64 r2, r7
    ldxdw r3, [r10 - 88]
    stxb [r2 + 12], r3
    ldxdw r7, [r10 - 56]
    mov64 r2, r1
    add64 r2, r7
    mov64 r3, 0
    stxb [r2 + 12], r3
    ldxdw r7, [r10 - 80]
    mov64 r2, r1
    add64 r2, r7
    stxb [r2 + 12], r3
    ldxdw r2, [r10 - 80]
    ldxdw r3, [r10 - 64]
    call rotate_left_asm
    ldxdw r1, [r10 - 40]
    ldxdw r2, [r10 - 56]
    ldxdw r3, [r10 - 64]
    call rotate_right_asm
    ja .Lremove_output_nil_nil

.Lremove_try_right_right:
    ldxdw r4, [r10 - 136]
    ldxdw r5, [r10 - 120]
    jeq r4, 0, .Lremove_try_right_left
    jne r5, 0, .Lremove_try_right_left

.Lremove_case_right_right:
    ldxdw r1, [r10 - 40]
    ldxdw r7, [r10 - 112]
    mov64 r2, r1
    add64 r2, r7
    mov64 r3, 0
    stxb [r2 + 12], r3
    ldxdw r7, [r10 - 56]
    mov64 r2, r1
    add64 r2, r7
    ldxdw r3, [r10 - 96]
    stxb [r2 + 12], r3
    ldxdw r7, [r10 - 80]
    mov64 r2, r1
    add64 r2, r7
    ldxdw r3, [r10 - 88]
    stxb [r2 + 12], r3
    ldxdw r2, [r10 - 56]
    ldxdw r3, [r10 - 64]
    call rotate_left_asm
    ja .Lremove_output_nil_nil

.Lremove_try_right_left:
    ldxdw r1, [r10 - 40]
    ldxdw r7, [r10 - 104]
    mov64 r2, r1
    add64 r2, r7
    ldxdw r3, [r10 - 88]
    stxb [r2 + 12], r3
    ldxdw r7, [r10 - 56]
    mov64 r2, r1
    add64 r2, r7
    mov64 r3, 0
    stxb [r2 + 12], r3
    ldxdw r7, [r10 - 80]
    mov64 r2, r1
    add64 r2, r7
    stxb [r2 + 12], r3
    ldxdw r2, [r10 - 80]
    ldxdw r3, [r10 - 64]
    call rotate_right_asm
    ldxdw r1, [r10 - 40]
    ldxdw r2, [r10 - 56]
    ldxdw r3, [r10 - 64]
    call rotate_left_asm
    ja .Lremove_output_nil_nil

.Lremove_sibling_black_no_red_children:
    ldxdw r1, [r10 - 40]
    ldxdw r8, [r10 - 80]
    jeq r8, r6, .Lremove_check_parent_color
    mov64 r4, r1
    add64 r4, r8
    mov64 r7, 1
    stxb [r4 + 12], r7

.Lremove_check_parent_color:
    ldxdw r7, [r10 - 88]
    mov64 r4, 0
    jne r7, r4, .Lremove_parent_was_red

    ldxdw r1, [r10 - 40]
    ldxdw r7, [r10 - 56]
    mov64 r4, r1
    add64 r4, r7
    ldxw r8, [r4 + 8]
    ldxdw r5, [r10 - 72]
    stxw [r5 + 0], r7
    stxw [r5 + 4], r8
    ja .Lremove_restore_and_exit

.Lremove_parent_was_red:
    ldxdw r1, [r10 - 40]
    ldxdw r7, [r10 - 56]
    mov64 r4, r1
    add64 r4, r7
    mov64 r8, 0
    stxb [r4 + 12], r8
    ja .Lremove_output_nil_nil

.Lremove_sibling_red:
    ldxdw r5, [r10 - 120]
    jeq r5, 0, .Lremove_sibling_red_right

.Lremove_sibling_red_left:
    ldxdw r1, [r10 - 40]
    ldxdw r2, [r10 - 56]
    ldxdw r3, [r10 - 64]
    call rotate_right_asm
    ja .Lremove_sibling_red_recolor_and_continue

.Lremove_sibling_red_right:
    ldxdw r1, [r10 - 40]
    ldxdw r2, [r10 - 56]
    ldxdw r3, [r10 - 64]
    call rotate_left_asm

.Lremove_sibling_red_recolor_and_continue:
    ldxdw r1, [r10 - 40]
    ldxdw r7, [r10 - 56]
    mov64 r4, r1
    add64 r4, r7
    mov64 r8, 1
    stxb [r4 + 12], r8
    ldxdw r7, [r10 - 80]
    mov64 r4, r1
    add64 r4, r7
    mov64 r8, 0
    stxb [r4 + 12], r8
    ldxdw r5, [r10 - 72]
    ldxdw r7, [r10 - 48]
    ldxdw r8, [r10 - 56]
    stxw [r5 + 0], r7
    stxw [r5 + 4], r8
    ja .Lremove_restore_and_exit

.Lremove_output_nil_nil:
    ldxdw r5, [r10 - 72]
    lddw r6, 4294967295
    stxw [r5 + 0], r6
    stxw [r5 + 4], r6

.Lremove_restore_and_exit:
    ldxdw r6, [r10 - 8]
    ldxdw r7, [r10 - 16]
    ldxdw r8, [r10 - 24]
    ldxdw r9, [r10 - 32]
    exit
