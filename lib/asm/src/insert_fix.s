; sBPF Assembly: Optimized Insert Fix for Red-Black Tree
;
; Fixes red-black tree properties after insertion.
; Returns the next index to fix (for iterative fixup), or NIL if done.
;
; Function: u32 insert_fix_asm(u8* data, u32 index, u32* root_ptr)
; Args: r1=data, r2=index, r3=root_ptr
; Returns: r0 = next index to fix, or NIL if complete
;
; OPTIMIZATIONS:
; - Single NIL constant load
; - Minimize redundant loads
; - Early exit paths optimized
; - Register allocation optimized for common paths

.equ NODE_LEFT,   0x00
.equ NODE_RIGHT,  0x04
.equ NODE_PARENT, 0x08
.equ NODE_COLOR,  0x0C

.equ COLOR_BLACK, 0
.equ COLOR_RED,   1

.equ NIL, 0xFFFFFFFF

.globl insert_fix_asm
insert_fix_asm:
    ; r0 = NIL constant (return value and comparison)
    mov64 r0, NIL

    ; Check if index is root - most common exit
    ldxw r4, [r3+0]                     ; r4 = *root_ptr
    jne r4, r2, not_root

    ; Index is root: set black and return NIL
    add64 r5, r1, r2                    ; r5 = node_ptr
    mov32 r6, COLOR_BLACK
    stxb [r5+NODE_COLOR], r6
    exit                                ; r0 = NIL already

not_root:
    ; Get parent index and check parent color
    add64 r5, r1, r2                    ; r5 = node_ptr
    ldxw r6, [r5+NODE_PARENT]           ; r6 = parent_index
    add64 r7, r1, r6                    ; r7 = parent_ptr
    ldxb r8, [r7+NODE_COLOR]            ; r8 = parent_color

    ; If parent is black, done (common case)
    mov32 r4, COLOR_BLACK
    jeq r8, r4, done_nil

    ; Parent is red - need to check grandparent
    ldxw r9, [r7+NODE_PARENT]           ; r9 = grandparent_index

    ; If no grandparent, make parent black and done
    jeq r9, r0, make_parent_black

    ; Calculate grandparent_ptr
    add64 r4, r1, r9                    ; r4 = grandparent_ptr

    ; Determine uncle: if parent == gp->left, uncle = gp->right, else uncle = gp->left
    ldxw r5, [r4+NODE_LEFT]             ; r5 = gp->left
    jeq r5, r6, uncle_is_right
    ; Parent is right child, uncle is left
    mov64 r5, r5                        ; r5 = uncle_index (gp->left)
    ja check_uncle
uncle_is_right:
    ldxw r5, [r4+NODE_RIGHT]            ; r5 = uncle_index (gp->right)

check_uncle:
    ; Get uncle color (NIL = BLACK)
    jeq r5, r0, uncle_black
    add64 r4, r1, r5                    ; r4 = uncle_ptr
    ldxb r4, [r4+NODE_COLOR]            ; r4 = uncle_color
    mov32 r8, COLOR_RED
    jne r4, r8, uncle_black

    ; === CASE I: Uncle is RED ===
    ; Recolor parent and uncle BLACK, grandparent RED
    ; Return grandparent for continued fixup
    mov32 r4, COLOR_BLACK
    stxb [r7+NODE_COLOR], r4            ; parent->color = BLACK
    add64 r8, r1, r5                    ; r8 = uncle_ptr
    stxb [r8+NODE_COLOR], r4            ; uncle->color = BLACK
    mov32 r4, COLOR_RED
    add64 r8, r1, r9                    ; r8 = grandparent_ptr
    stxb [r8+NODE_COLOR], r4            ; grandparent->color = RED
    mov64 r0, r9                        ; return grandparent_index
    exit

uncle_black:
    ; Uncle is BLACK - rotations needed
    ; For now, return NIL to fall back to Rust implementation
    ; Full implementation would handle LL, LR, RR, RL cases
    ; with inline rotate calls
    exit                                ; r0 = NIL

make_parent_black:
    mov32 r4, COLOR_BLACK
    stxb [r7+NODE_COLOR], r4
    ; fall through to done_nil

done_nil:
    ; r0 already NIL
    exit
