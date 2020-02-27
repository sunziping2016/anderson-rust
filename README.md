# Anderson Rust

## How to Use

Create a file named `input.txt` with the following content:

```c
p = &a;
q = &b;
r = &c;
s = p;
*p = q;
t = *p;
*s = r;
``` 

The semicolon can be omitted. Then run the following command.

```bash
cargo run --package anderson-rust --bin anderson-rust -- input.txt output.gv
```

Then a Graphviz named `output.gv` is generated. You can view it with xdot or
render it into svg or png.
