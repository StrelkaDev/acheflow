(module
  (import "acheflow" "emit_json" (func $emit (param i32 i32)))
  (import "acheflow" "matches" (func $matches (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (global $count (mut i32) (i32.const 0))
  (data (i32.const 0) "\7b\22\74\79\70\65\22\3a\22\70\61\6e\65\6c\22\2c\22\70\61\6e\65\6c\22\3a\7b\22\65\78\74\65\6e\73\69\6f\6e\5f\69\64\22\3a\22\73\61\6d\70\6c\65\22\2c\22\74\69\74\6c\65\22\3a\22\53\61\6d\70\6c\65\20\65\78\74\65\6e\73\69\6f\6e\22\2c\22\77\69\64\67\65\74\73\22\3a\5b\7b\22\74\79\70\65\22\3a\22\6d\65\74\72\69\63\22\2c\22\6c\61\62\65\6c\22\3a\22\41\63\74\69\6f\6e\73\22\2c\22\76\61\6c\75\65\22\3a\22\30\22\7d\2c\7b\22\74\79\70\65\22\3a\22\62\75\74\74\6f\6e\22\2c\22\69\64\22\3a\22\69\6e\63\72\65\6d\65\6e\74\22\2c\22\6c\61\62\65\6c\22\3a\22\52\75\6e\20\61\63\74\69\6f\6e\22\7d\5d\7d\7d")
  (data (i32.const 193) "\7b\22\2f\74\79\70\65\22\3a\22\61\63\74\69\6f\6e\22\2c\22\2f\61\63\74\69\6f\6e\5f\69\64\22\3a\22\69\6e\63\72\65\6d\65\6e\74\22\7d")
  (func (export "handle")
    i32.const 193 i32.const 43 call $matches
    if
      global.get $count i32.const 1 i32.add i32.const 10 i32.rem_u global.set $count
      i32.const 131 global.get $count i32.const 48 i32.add i32.store8
    end
    i32.const 0 i32.const 193 call $emit)
)
