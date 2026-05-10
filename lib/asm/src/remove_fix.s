; sBPF Assembly: Optimized Remove Fix for Red-Black Tree
;
; Fixes red-black tree properties after deletion.
; Handles the double-black node case.
;
; Function: void remove_fix_asm(u8* data, u32 current, u32 parent, u32* root_ptr,
;                               u32* out_current, u32* out_parent)
; Args: r1=data, r2=current, r3=parent, r4=root_ptr, r5=out_current
; Stack: [r10-8]=out_parent
;
; OPTIMIZATIONS:
; - Single NIL load
; - Optimized common path (current is root)
; - Minimized register spills

.equ NODE_LEFT,   0x00
.equ NODE_RIGHT,  0x04
.equ NODE_PARENT, 0x08
.equ NODE_COLOR,  0x0C

.equ COLOR_BLACK, 0
.equ COLOR_RED,   1

.equ NIL, 0xFFFFFFFF

.globl remove_fix_asm
remove_fix_asm:
    ; r0 = NIL constant
    mov64 r0, NIL

    ; Check if current is root - common early exit
    ldxw r6, [r4+0]                     ; r6 = *root_ptr
    jne r6, r2, not_at_root

    ; At root - done, return (NIL, NIL)
    stxw [r5+0], r0                     ; *out_current = NIL
    ldxdw r6, [r10-8]                   ; r6 = out_parent ptr
    stxw [r6+0], r0                     ; *out_parent = NIL
    exit

not_at_root:
    ; Calculate parent_ptr and get parent info
    add64 r6, r1, r3                    ; r6 = parent_ptr
    ldxw r7, [r6+NODE_LEFT]             ; r7 = parent->left

    ; Determine sibling: if current is left child, sibling is right, else left
    jeq r7, r2, current_is_left
    ; Current is right child or NIL on right, sibling is left
    mov64 r8, r7                        ; r8 = sibling_index
    ja have_sibling
current_is_left:
    ldxw r8, [r6+NODE_RIGHT]            ; r8 = sibling_index

have_sibling:
    ; Get sibling color (NIL = BLACK)
    jeq r8, r0, sibling_black
    add64 r9, r1, r8                    ; r9 = sibling_ptr
    ldxb r7, [r9+NODE_COLOR]            ; r7 = sibling_color
    mov32 r6, COLOR_BLACK
    jeq r7, r6, sibling_black

    ; === Sibling is RED (Case 3c) ===
    ; Need rotation and recoloring, continue with current,parent
    stxw [r5+0], r2                     ; *out_current = current
    ldxdw r6, [r10-8]
    stxw [r6+0], r3                     ; *out_parent = parent
    exit

sibling_black:
    ; Sibling is BLACK
    ; Check if sibling has red children
    jeq r8, r0, no_sibling

    add64 r9, r1, r8                    ; r9 = sibling_ptr
    ldxw r6, [r9+NODE_LEFT]             ; r6 = sibling->left
    ldxw r7, [r9+NODE_RIGHT]            ; r7 = sibling->right

    ; Check left child color
    jeq r6, r0, check_right_child
    add64 r6, r1, r6
    ldxb r6, [r6+NODE_COLOR]
    mov32 r9, COLOR_RED
    jeq r6, r9, has_red_child

check_right_child:
    jeq r7, r0, no_red_children
    add64 r7, r1, r7
    ldxb r7, [r7+NODE_COLOR]
    mov32 r9, COLOR_RED
    jeq r7, r9, has_red_child

no_red_children:
no_sibling:
    ; Case 3b: Sibling is BLACK with no red children
    ; Recolor sibling RED (if exists)
    jeq r8, r0, check_parent_color
    add64 r9, r1, r8
    mov32 r7, COLOR_RED
    stxb [r9+NODE_COLOR], r7

check_parent_color:
    ; Get parent color
    add64 r6, r1, r3                    ; r6 = parent_ptr
    ldxb r7, [r6+NODE_COLOR]
    mov32 r9, COLOR_BLACK
    jne r7, r9, parent_was_red

    ; Parent was BLACK - continue fixup up the tree
    stxw [r5+0], r3                     ; *out_current = parent
    ldxw r7, [r6+NODE_PARENT]           ; parent's parent
    ldxdw r6, [r10-8]
    stxw [r6+0], r7                     ; *out_parent = parent's parent
    exit

parent_was_red:
    ; Parent was RED - make it BLACK and done
    mov32 r7, COLOR_BLACK
    stxb [r6+NODE_COLOR], r7
    stxw [r5+0], r0                     ; *out_current = NIL
    ldxdw r6, [r10-8]
    stxw [r6+0], r0                     ; *out_parent = NIL
    exit

has_red_child:
    ; Case 3a: Sibling is BLACK with red child
    ; Complex case - fall back to Rust by returning (NIL, NIL)
    stxw [r5+0], r0
    ldxdw r6, [r10-8]
    stxw [r6+0], r0
    exit
