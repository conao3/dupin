(module
  (memory (export "memory") 8)
  (global $next (mut i32) (i32.const 1024))

  (func (export "dupin_abi_version") (result i32)
    i32.const 1)

  (func (export "dupin_alloc") (param $size i32) (result i32)
    (local $start i32)
    (local $end i32)
    local.get $size
    i32.const 0
    i32.lt_s
    if
      i32.const 0
      return
    end
    global.get $next
    local.tee $start
    local.get $size
    i32.add
    local.tee $end
    local.get $start
    i32.lt_u
    if
      i32.const 0
      return
    end
    local.get $end
    i32.const 300000
    i32.gt_u
    if
      i32.const 0
      return
    end
    local.get $end
    global.set $next
    local.get $start)

  (func (export "dupin_query")
    (param $input i32) (param $input_len i32)
    (param $output i32) (param $output_cap i32)
    (result i32)
    local.get $output_cap
    i32.const 114
    i32.lt_u
    if
      i32.const -1
      return
    end
    local.get $output
    i32.const 400000
    i32.const 114
    memory.copy
    i32.const 114)

  (data (i32.const 400000)
    "{\22version\22:3,\22request_id\22:\22minimal-probe-1\22,\22generation\22:\221\22,\22outcome\22:{\22kind\22:\22results\22,\22items\22:[],\22state\22:null}}"))
