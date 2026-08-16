document.documentElement.classList.add("js");

const translations = {
  zh: {
    pageTitle: "Alter — 让每次抵达都更快",
    pageDescription:
      "Alter 是面向 Hyprland 与 Wayland 的快速、键盘优先启动器。应用、文件、剪贴板和工作流，一个入口即可抵达。",
    skip: "跳到主要内容",
    navFeatures: "能力",
    navExperience: "体验",
    navInstall: "安装",
    heroEyebrow: "为 Hyprland 与 Wayland 而生",
    heroTitleOne: "少一点寻找，",
    heroTitleTwo: "多一点抵达。",
    heroLead:
      "应用、文件、剪贴板与自动化工作流，全部收进一个轻快的 GTK4 浮层。手不离键盘，想法就不会断线。",
    installNow: "立即安装",
    viewSource: "查看源码",
    modeApps: "应用",
    modeFiles: "文件",
    modeClipboard: "剪贴板",
    modeWeb: "网页",
    select: "选择",
    open: "打开",
    close: "关闭",
    summon: "随时唤起",
    scrollExplore: "继续探索",
    featureKicker: "ONE PLACE. EVERYWHERE.",
    featureTitle: "一个入口，接住所有念头。",
    featureLead:
      "Alter 把高频操作压缩成一次搜索。不需要记住菜单在哪里，只需要记住你想做什么。",
    unifiedTitle: "搜索，不止应用",
    unifiedBody:
      "用一个字符切换上下文：应用、文件、剪贴板或网页。结果在你完成输入前就已经出现。",
    clipboardTitle: "剪贴板，有记忆",
    clipboardBody:
      "搜索、预览、固定或隐藏历史内容。文字留在本机，隐私也留在你手里。",
    workflowTitle: "把步骤折成一次回车",
    workflowBody:
      "Workflow、Quick Links 与 Snippets，让重复操作变成可搜索的快捷方式。",
    calculatorTitle: "答案，不用离开桌面",
    calculatorBody:
      "直接输入算式，按下回车复制结果；需要搜索网页时，也只差一个前缀。",
    nativeTitle: "原生，所以轻快",
    nativeBody:
      "Rust + GTK4 构建，融入 Wayland 桌面；后台常驻，快捷键一按即现。",
    localTitle: "本地优先，默认克制",
    localBody:
      "搜索和剪贴板数据都留在你的设备上。扩展命令以参数数组执行，不偷偷经过 shell。",
    experienceKicker: "KEYBOARD FIRST",
    experienceTitle: "快到像没打开过窗口。",
    experienceLead:
      "唤起、搜索、执行，然后消失。Alter 尊重你的注意力，也尊重你的桌面。",
    flowSummon: "从任何位置唤起",
    flowSearch: "输入即搜索",
    flowDone: "执行，继续专注",
    shotCaption: "一致、安静、随叫随到。",
    shotDetail: "深浅主题 · 中英文界面 · Waybar 托盘",
    installKicker: "GET ALTER",
    installTitle: "一条命令，开始提速。",
    installLead:
      "面向运行 Hyprland 的 Wayland Linux 桌面。推荐通过 AUR 安装，也可以从源码构建。",
    fromSource: "源码",
    aurComment: "# 使用你偏好的 AUR helper",
    sourceComment: "# 克隆并构建 release 版本",
    copy: "复制",
    copied: "已复制",
    ready: "一切就绪",
    ctaTitle: "让桌面跟上你的速度。",
    ctaBody: "免费、开源，并且仍在变得更好。",
    viewRelease: "查看最新版本",
    footerMade: "为更快的 Linux 桌面而造。",
    menuOpen: "打开导航",
    menuClose: "关闭导航",
    switchLanguage: "Switch to English",
    result: "条结果",
    types: { app: "应用", file: "文件", clipboard: "剪贴板", web: "网页" },
  },
  en: {
    pageTitle: "Alter — Every action, closer",
    pageDescription:
      "Alter is a fast, keyboard-first launcher for Hyprland and Wayland. Apps, files, clipboard history, and workflows in one place.",
    skip: "Skip to main content",
    navFeatures: "Features",
    navExperience: "Experience",
    navInstall: "Install",
    heroEyebrow: "Built for Hyprland & Wayland",
    heroTitleOne: "Less searching.",
    heroTitleTwo: "More arriving.",
    heroLead:
      "Apps, files, clipboard history, and automated workflows—inside one nimble GTK4 overlay. Keep your hands on the keyboard and your thoughts in motion.",
    installNow: "Install Alter",
    viewSource: "View source",
    modeApps: "Apps",
    modeFiles: "Files",
    modeClipboard: "Clipboard",
    modeWeb: "Web",
    select: "Select",
    open: "Open",
    close: "Close",
    summon: "Summon anywhere",
    scrollExplore: "Keep exploring",
    featureKicker: "ONE PLACE. EVERYWHERE.",
    featureTitle: "One place for every intent.",
    featureLead:
      "Alter compresses frequent actions into one search. Forget where the menu lives—just remember what you want to do.",
    unifiedTitle: "Search beyond apps",
    unifiedBody:
      "Switch context with one character: apps, files, clipboard, or the web. Results appear before you finish typing.",
    clipboardTitle: "A clipboard with memory",
    clipboardBody:
      "Search, preview, pin, or hide past items. Your text stays local, and your privacy stays yours.",
    workflowTitle: "Turn many steps into Enter",
    workflowBody:
      "Workflows, Quick Links, and Snippets make repeat actions instantly searchable.",
    calculatorTitle: "Answers without context switching",
    calculatorBody:
      "Type an expression and press Enter to copy the result. Web search is only one prefix away, too.",
    nativeTitle: "Native means nimble",
    nativeBody:
      "Built with Rust and GTK4 for the Wayland desktop. Resident in the background, instant on your shortcut.",
    localTitle: "Local-first by default",
    localBody:
      "Search and clipboard data remain on your device. Extension commands run as argument arrays, never secretly through a shell.",
    experienceKicker: "KEYBOARD FIRST",
    experienceTitle: "So fast it barely feels open.",
    experienceLead:
      "Summon, search, act, and disappear. Alter respects both your attention and your desktop.",
    flowSummon: "Summon from anywhere",
    flowSearch: "Type to search",
    flowDone: "Act and stay focused",
    shotCaption: "Consistent, quiet, always ready.",
    shotDetail: "Light & dark · English & Chinese · Waybar tray",
    installKicker: "GET ALTER",
    installTitle: "One command. Faster already.",
    installLead:
      "For Wayland Linux desktops running Hyprland. Install from the AUR, or build directly from source.",
    fromSource: "Source",
    aurComment: "# Use your preferred AUR helper",
    sourceComment: "# Clone and build the release binary",
    copy: "Copy",
    copied: "Copied",
    ready: "ready when you are",
    ctaTitle: "Let your desktop keep up.",
    ctaBody: "Free, open source, and getting better.",
    viewRelease: "View latest release",
    footerMade: "Made for a faster Linux desktop.",
    menuOpen: "Open navigation",
    menuClose: "Close navigation",
    switchLanguage: "切换到中文",
    result: "results",
    types: { app: "App", file: "File", clipboard: "Clipboard", web: "Web" },
  },
};

const demoModes = {
  apps: {
    query: "chrome",
    results: [
      {
        icon: "GC",
        title: "Google Chrome",
        subtitle: "Web Browser",
        type: "app",
        color: "#7aa2f7",
      },
      {
        icon: "GM",
        title: "Gmail",
        subtitle: "Application",
        type: "app",
        color: "#ef6b73",
      },
      {
        icon: "GD",
        title: "Google Drive",
        subtitle: "Application",
        type: "app",
        color: "#6ee7b7",
      },
    ],
  },
  files: {
    query: "f readme",
    results: [
      {
        icon: "MD",
        title: "README.md",
        subtitle: "~/projects/alter",
        type: "file",
        color: "#60a5fa",
      },
      {
        icon: "中",
        title: "README.zh-CN.md",
        subtitle: "~/projects/alter",
        type: "file",
        color: "#a78bfa",
      },
      {
        icon: "RS",
        title: "src/main.rs",
        subtitle: "~/projects/alter",
        type: "file",
        color: "#f59e78",
      },
    ],
  },
  clipboard: {
    query: "c cargo",
    results: [
      {
        icon: ">_",
        title: "cargo build --release",
        subtitle: "Copied 2 minutes ago",
        type: "clipboard",
        color: "#a78bfa",
      },
      {
        icon: "#",
        title: "cargo fmt --all -- --check",
        subtitle: "Copied yesterday",
        type: "clipboard",
        color: "#67e8f9",
      },
      {
        icon: "↗",
        title: "https://github.com/yuukidach/alter",
        subtitle: "Pinned",
        type: "clipboard",
        color: "#6ee7b7",
      },
    ],
  },
  web: {
    query: "? wayland launcher",
    results: [
      {
        icon: "D",
        title: "Search DuckDuckGo",
        subtitle: "wayland launcher",
        type: "web",
        color: "#f59e78",
      },
      {
        icon: "G",
        title: "Search Google",
        subtitle: "wayland launcher",
        type: "web",
        color: "#7aa2f7",
      },
      {
        icon: "B",
        title: "Search Bing",
        subtitle: "wayland launcher",
        type: "web",
        color: "#67e8f9",
      },
    ],
  },
};

let currentLanguage = "en";
let currentMode = "apps";

try {
  const savedLanguage = localStorage.getItem("alter-language");
  if (savedLanguage === "zh" || savedLanguage === "en") {
    currentLanguage = savedLanguage;
  }
} catch (_) {
  // Storage can be unavailable in strict privacy modes; English remains the default.
}

const queryNode = document.querySelector("[data-demo-query]");
const resultsNode = document.querySelector("[data-demo-results]");
const resultCountNode = document.querySelector("[data-demo-count]");
const languageButton = document.querySelector("[data-language-toggle]");
const menuButton = document.querySelector("[data-menu-toggle]");
const menu = document.querySelector("[data-menu]");

function renderDemo(modeName, animateQuery = true) {
  const mode = demoModes[modeName];
  const locale = translations[currentLanguage];
  currentMode = modeName;

  document.querySelectorAll("[data-mode]").forEach((button) => {
    const active = button.dataset.mode === modeName;
    button.classList.toggle("active", active);
    button.setAttribute("aria-selected", String(active));
  });

  if (animateQuery) {
    queryNode.animate(
      [
        { opacity: 0, transform: "translateY(4px)" },
        { opacity: 1, transform: "translateY(0)" },
      ],
      { duration: 220, easing: "ease-out" },
    );
  }
  queryNode.textContent = mode.query;
  resultsNode.replaceChildren();

  mode.results.forEach((result, index) => {
    const row = document.createElement("div");
    row.className = `result-row${index === 0 ? " selected" : ""}`;
    row.style.setProperty("--result-color", result.color);

    const icon = document.createElement("span");
    icon.className = "result-icon";
    icon.textContent = result.icon;

    const copy = document.createElement("div");
    copy.className = "result-copy";
    const title = document.createElement("b");
    title.textContent = result.title;
    const subtitle = document.createElement("small");
    subtitle.textContent = result.subtitle;
    copy.append(title, subtitle);

    const type = document.createElement("em");
    type.className = "result-type";
    type.textContent = locale.types[result.type];

    const arrow = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    arrow.classList.add("result-arrow");
    arrow.setAttribute("viewBox", "0 0 20 20");
    arrow.setAttribute("aria-hidden", "true");
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", "m7 4 6 6-6 6");
    arrow.append(path);

    row.append(icon, copy, type, arrow);
    resultsNode.append(row);
  });

  resultCountNode.textContent = `${mode.results.length} ${locale.result}`;
}

function applyLanguage(language) {
  currentLanguage = language;
  const locale = translations[language];
  document.documentElement.lang = language === "zh" ? "zh-CN" : "en";
  document.documentElement.dataset.language = language;
  document.title = locale.pageTitle;
  document.querySelector('meta[name="description"]').content =
    locale.pageDescription;

  document.querySelectorAll("[data-i18n]").forEach((node) => {
    const key = node.dataset.i18n;
    if (typeof locale[key] === "string") node.textContent = locale[key];
  });

  languageButton.innerHTML =
    language === "zh"
      ? '<span class="language-active">中</span><span class="language-divider">/</span><span>EN</span>'
      : '<span>中</span><span class="language-divider">/</span><span class="language-active">EN</span>';
  languageButton.setAttribute("aria-label", locale.switchLanguage);
  menuButton.setAttribute("aria-label", locale.menuOpen);
  renderDemo(currentMode, false);
}

languageButton.addEventListener("click", () => {
  const nextLanguage = currentLanguage === "zh" ? "en" : "zh";
  try {
    localStorage.setItem("alter-language", nextLanguage);
  } catch (_) {
    // The language still changes for this session when storage is unavailable.
  }
  applyLanguage(nextLanguage);
});

document.querySelectorAll("[data-mode]").forEach((button) => {
  button.addEventListener("click", () => {
    renderDemo(button.dataset.mode);
    restartDemoRotation();
  });
});

const modeNames = Object.keys(demoModes);
let rotationTimer;

function restartDemoRotation() {
  window.clearInterval(rotationTimer);
  rotationTimer = window.setInterval(() => {
    const nextIndex = (modeNames.indexOf(currentMode) + 1) % modeNames.length;
    renderDemo(modeNames[nextIndex]);
  }, 4800);
}

const demo = document.querySelector(".hero-demo");
demo.addEventListener("mouseenter", () => window.clearInterval(rotationTimer));
demo.addEventListener("mouseleave", restartDemoRotation);
demo.addEventListener("focusin", () => window.clearInterval(rotationTimer));
demo.addEventListener("focusout", restartDemoRotation);

document.querySelectorAll("[data-install-tab]").forEach((tab) => {
  tab.addEventListener("click", () => {
    const target = tab.dataset.installTab;
    document.querySelectorAll("[data-install-tab]").forEach((candidate) => {
      const active = candidate === tab;
      candidate.classList.toggle("active", active);
      candidate.setAttribute("aria-selected", String(active));
    });
    document.querySelectorAll("[data-install-panel]").forEach((panel) => {
      const active = panel.dataset.installPanel === target;
      panel.classList.toggle("active", active);
      panel.hidden = !active;
    });
  });
});

async function copyText(value) {
  if (navigator.clipboard && window.isSecureContext) {
    await navigator.clipboard.writeText(value);
    return;
  }

  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  document.body.append(textarea);
  textarea.select();
  document.execCommand("copy");
  textarea.remove();
}

document.querySelectorAll("[data-copy]").forEach((button) => {
  button.addEventListener("click", async () => {
    try {
      await copyText(button.dataset.copy);
      const label = button.querySelector("span");
      const previous = label.textContent;
      label.textContent = translations[currentLanguage].copied;
      button.classList.add("copied");
      window.setTimeout(() => {
        label.textContent = translations[currentLanguage].copy || previous;
        button.classList.remove("copied");
      }, 1600);
    } catch (_) {
      // Keep the command selectable if browser clipboard permissions are denied.
    }
  });
});

function closeMenu() {
  menu.classList.remove("open");
  menuButton.setAttribute("aria-expanded", "false");
  menuButton.setAttribute("aria-label", translations[currentLanguage].menuOpen);
}

menuButton.addEventListener("click", () => {
  const willOpen = !menu.classList.contains("open");
  menu.classList.toggle("open", willOpen);
  menuButton.setAttribute("aria-expanded", String(willOpen));
  menuButton.setAttribute(
    "aria-label",
    willOpen
      ? translations[currentLanguage].menuClose
      : translations[currentLanguage].menuOpen,
  );
});

menu
  .querySelectorAll("a")
  .forEach((link) => link.addEventListener("click", closeMenu));
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeMenu();
});
document.addEventListener("click", (event) => {
  if (!menu.contains(event.target) && !menuButton.contains(event.target))
    closeMenu();
});

const header = document.querySelector("[data-header]");
function updateHeader() {
  header.classList.toggle("scrolled", window.scrollY > 18);
}
window.addEventListener("scroll", updateHeader, { passive: true });
updateHeader();

if ("IntersectionObserver" in window) {
  const observer = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (entry.isIntersecting) {
          entry.target.classList.add("visible");
          observer.unobserve(entry.target);
        }
      });
    },
    { threshold: 0.12, rootMargin: "0px 0px -4%" },
  );
  document
    .querySelectorAll(".reveal")
    .forEach((node) => observer.observe(node));
} else {
  document
    .querySelectorAll(".reveal")
    .forEach((node) => node.classList.add("visible"));
}

const pointerGlow = document.querySelector(".pointer-glow");
let pointerFrame;
window.addEventListener(
  "pointermove",
  (event) => {
    if (event.pointerType === "touch") return;
    window.cancelAnimationFrame(pointerFrame);
    pointerFrame = window.requestAnimationFrame(() => {
      pointerGlow.style.left = `${event.clientX}px`;
      pointerGlow.style.top = `${event.clientY}px`;
    });
  },
  { passive: true },
);

applyLanguage(currentLanguage);
restartDemoRotation();
