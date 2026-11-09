const menuButton = document.querySelector(".menu-button");
const siteNavigation = document.querySelector("#site-nav");

menuButton?.addEventListener("click", () => {
  const isOpen = menuButton.getAttribute("aria-expanded") === "true";
  menuButton.setAttribute("aria-expanded", String(!isOpen));
  siteNavigation?.classList.toggle("open", !isOpen);
});

siteNavigation?.querySelectorAll("a").forEach((link) => {
  link.addEventListener("click", () => {
    menuButton?.setAttribute("aria-expanded", "false");
    siteNavigation.classList.remove("open");
  });
});

const year = document.querySelector("#year");
if (year) year.textContent = String(new Date().getFullYear());
