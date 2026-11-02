"""A real WASM action/panel example: the counter is a mutable WASM global."""
import json
from pathlib import Path
panel={'type':'panel','panel':{'extension_id':'sample','title':'Sample extension','widgets':[{'type':'metric','label':'Actions','value':'0'},{'type':'button','id':'increment','label':'Run action'}]}}
raw=json.dumps(panel,separators=(',',':')).encode()
condition=json.dumps({'/type':'action','/action_id':'increment'},separators=(',',':')).encode()
position=raw.index(b'"value":"0"')+len(b'"value":"')
def escaped(value):return ''.join('\\%02x'%b for b in value)
wat=f'''(module
  (import "acheflow" "emit_json" (func $emit (param i32 i32)))
  (import "acheflow" "matches" (func $matches (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (global $count (mut i32) (i32.const 0))
  (data (i32.const 0) "{escaped(raw)}")
  (data (i32.const {len(raw)}) "{escaped(condition)}")
  (func (export "handle")
    i32.const {len(raw)} i32.const {len(condition)} call $matches
    if
      global.get $count i32.const 1 i32.add i32.const 10 i32.rem_u global.set $count
      i32.const {position} global.get $count i32.const 48 i32.add i32.store8
    end
    i32.const 0 i32.const {len(raw)} call $emit)
)
'''
Path(__file__).with_name('sample.wat').write_text(wat)
