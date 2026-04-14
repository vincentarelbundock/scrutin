# Intentionally messy code to trigger ruff lint warnings.

import os, sys  # noqa: F401 — unused imports (F401), multi-import (E401)
import json  # noqa: F401

# Unused variable
def greet(name):
    x = 42  # F841
    return f"hello, {name}"

# Type comparison with isinstance instead of type()
def check_type(x):
    if type(x) == int:  # E721: use isinstance()
        return "integer"
    return "other"

# Bare except
def risky_divide(x, y):
    try:
        return x / y
    except:  # E722: bare except
        return None

# Mutable default argument
def append_item(item, lst=[]):  # B006
    lst.append(item)
    return lst

# f-string without placeholders
def greeting():
    return f"hello world"  # F541

# Yoda condition
def is_positive(x):
    if 0 < x:  # SIM300 (if selected)
        return True
    return False

# dict.get with None default (explicit None is redundant)
def lookup(d, key):
    return d.get(key, None)  # SIM910

# Needless else after return
def classify(x):
    if x > 0:
        return "positive"
    else:  # RET505
        return "negative"

# Open without context manager
def read_file(path):
    f = open(path)  # SIM115
    data = f.read()
    f.close()
    return data
