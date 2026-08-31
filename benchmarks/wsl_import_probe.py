import sys, time

print("== per-module import cost ==")
for mod in ["rich", "pymupdf", "docx", "markdown", "striprtf", "edge_tts", "platformdirs", "lue", "lue.reader", "lue.ui"]:
    t0 = time.perf_counter()
    try:
        __import__(mod)
        print(f"{mod:16} {(time.perf_counter()-t0)*1000:6.0f} ms")
    except Exception as e:
        print(f"{mod:16} FAIL {e}")

print("== full import ==")
t0 = time.perf_counter()
import lue, lue.reader, lue.ui
print(f"import lue.reader/ui: {(time.perf_counter()-t0)*1000:.0f} ms")