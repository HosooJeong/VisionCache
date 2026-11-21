"""A fixed Windows GUI and a local oracle for the stock-entry tasks."""

import ctypes
from dataclasses import dataclass
import random
import tkinter as tk

from PIL import ImageGrab

WIDTH, HEIGHT = 1024, 640
BACKGROUND = "#f0f4f8"
CARD = "#ffffff"
INK = "#16283c"
MUTED = "#4c6077"
ACCENT = "#2258a6"


@dataclass(frozen=True)
class Task:
    sku: str
    quantity: str
    preferred_bin: str
    active_bin: str
    receipt: str

    @property
    def instruction(self):
        return (
            f"Validate and confirm a stock intake of SKU {self.sku}, quantity "
            f"{self.quantity}, using preferred bin {self.preferred_bin}. "
            "If the app rejects that bin, use the replacement shown in the app. "
            "Read the receipt code from the screen, enter it, and click Confirm intake."
        )


def make_task(scenario, seed, repetition):
    rng = random.Random(f"{seed}/{scenario}/{repetition}")
    bins = rng.sample(range(100, 999), 2)
    preferred, alternate = (f"B-{number}" for number in bins)
    return Task(
        sku=f"SKU-{rng.randrange(10000, 100000)}",
        quantity=str(rng.randrange(2, 90)),
        preferred_bin=preferred,
        active_bin=alternate if scenario == "recover-bin" else preferred,
        receipt="".join(rng.choices("346789ACDEFGHJKMNPQRTUVWXY", k=7)),
    )


class FixtureClosed(Exception):
    pass


class Fixture:
    def __init__(self):
        # Keep Tk widget coordinates and captured pixels in the same DPI space.
        set_dpi_context = getattr(ctypes.windll.user32, "SetProcessDpiAwarenessContext", None)
        if set_dpi_context is not None:
            set_dpi_context(ctypes.c_void_p(-4))  # PER_MONITOR_AWARE_V2
        self.root = tk.Tk()
        self.root.title("VisionCache | Stock intake")
        self.root.geometry(f"{WIDTH}x{HEIGHT}")
        self.root.resizable(False, False)
        self.root.configure(bg=BACKGROUND)
        self.closed = False
        self.root.protocol("WM_DELETE_WINDOW", self.close)
        self.root.bind("<Escape>", lambda _event: self.close())
        self.controls = {}
        self.task = None
        self.validated = None
        self.saved = None

        self.label("Stock intake", 46, 30, size=28, bg=BACKGROUND, bold=True)
        self.label("Inventory  /  Receiving", 48, 89, bg=BACKGROUND, fg=MUTED)
        tk.Frame(self.root, bg=CARD).place(x=42, y=139, width=940, height=453)
        self.label("Intake details", 72, 160, size=17, bold=True)
        self.label("Validation & receipt", 618, 160, size=17, bold=True)
        self.label("1. Check the item and storage bin.", 618, 205, fg=MUTED)
        self.label("2. Use the receipt code to confirm.", 618, 232, fg=MUTED)
        self.entry("sku", "SKU", 72, 224, tab_width=350)
        self.entry("quantity", "Quantity", 72, 306, tab_width=350)
        self.entry("bin", "Storage bin", 72, 388, tab_width=350)
        self.button("validate", "Validate intake", self.validate, 72, 494)
        self.message = self.label("Waiting for validation", 618, 291, bold=True)
        self.detail = self.label("", 618, 325, size=12)
        self.entry("receipt", "Receipt code", 618, 388, tab_width=285)
        self.button("confirm", "Confirm intake", self.confirm, 618, 494)
        self.update()

    def label(self, text, x, y, *, size=15, bg=CARD, fg=INK, bold=False):
        font = ("Segoe UI", -size, "bold" if bold else "normal")
        label = tk.Label(self.root, text=text, bg=bg, fg=fg, font=font,
                         borderwidth=0, padx=0, pady=0)
        label.place(x=x, y=y)
        return label

    def entry(self, name, label, x, y, tab_width):
        self.label(label, x, y)
        entry = tk.Entry(self.root, font=("Segoe UI", -18), relief="flat",
                         bg="#f6f8fb", fg=INK, highlightthickness=1,
                         highlightbackground="#aab9ca", highlightcolor=ACCENT,
                         insertofftime=0, selectbackground=ACCENT)
        entry.place(x=x, y=y + 27, width=tab_width, height=37)
        self.controls[name] = entry

    def button(self, name, label, command, x, y):
        button = tk.Button(self.root, text=label, command=command,
                           font=("Segoe UI", -16, "bold"), bg=ACCENT, fg=CARD,
                           activebackground="#184783", activeforeground=CARD,
                           relief="flat", highlightthickness=0, takefocus=False)
        button.place(x=x, y=y, width=168, height=42)
        self.controls[name] = button

    def status(self, message, detail="", color=INK):
        self.message.configure(text=message, fg=color)
        self.detail.configure(text=detail, fg=color)

    def close(self):
        if not self.closed:
            self.closed = True
            self.root.destroy()

    def update(self):
        if self.closed:
            raise FixtureClosed("Benchmark window was closed.")
        self.root.update()
        if self.closed:
            raise FixtureClosed("Benchmark window was closed.")

    def reset(self, task):
        self.task = task
        self.validated = None
        self.saved = None
        for control in self.controls.values():
            if isinstance(control, tk.Entry):
                control.delete(0, tk.END)
        self.root.focus_set()
        self.status("Waiting for validation")
        self.update()

    def values(self):
        return tuple(self.controls[name].get().strip()
                     for name in ("sku", "quantity", "bin"))

    def validate(self):
        sku, quantity, storage_bin = self.values()
        self.validated = None
        if (sku, quantity) != (self.task.sku, self.task.quantity):
            self.status("Check the SKU and quantity", "One or both values do not match.", "#ae292e")
        elif storage_bin != self.task.active_bin:
            self.status("Storage bin unavailable", f"Replacement bin: {self.task.active_bin}", "#ae292e")
        else:
            self.validated = self.values()
            self.status("Intake validated", f"Receipt code: {self.task.receipt}", "#216145")

    def confirm(self):
        values = self.values()
        if values != self.validated:
            self.status("Validate the intake first", color="#ae292e")
        elif self.controls["receipt"].get().strip() != self.task.receipt:
            self.status("Receipt code did not match", f"Receipt code: {self.task.receipt}", "#ae292e")
        else:
            self.saved = (*values, self.task.receipt)
            self.status("Intake confirmed", "The stock entry was saved.", "#216145")

    def succeeded(self):
        return self.saved == (self.task.sku, self.task.quantity,
                              self.task.active_bin, self.task.receipt)

    def capture(self):
        self.update()
        image = ImageGrab.grab(window=self.root.winfo_id())
        if image.size != (WIDTH, HEIGHT) or image.getpixel((4, 4)) != (240, 244, 248):
            raise RuntimeError(f"Unexpected fixture capture {image.size}. Use an interactive Windows desktop.")
        return image

    def apply(self, action):
        x, y = action["x"], action["y"]
        for control in self.controls.values():
            left, top = control.winfo_x(), control.winfo_y()
            if not (left <= x < left + control.winfo_width()
                    and top <= y < top + control.winfo_height()):
                continue
            if action["action"] == "fill" and isinstance(control, tk.Entry):
                control.focus_set()
                control.delete(0, tk.END)
                control.insert(0, action["text"])
                self.update()
                return "Text entered into target."
            if action["action"] == "click" and isinstance(control, tk.Button):
                control.invoke()
                self.update()
                return "Button clicked."
            break
        return "No compatible control at those coordinates."