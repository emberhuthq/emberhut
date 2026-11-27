import { describe, expect, it } from "vitest";
import { blockedList, hostOf, isBlocked, isNewTabPage, isTabBlocked } from "./scope";

describe("scope", () => {
  it("blocks password managers, account security and the Web Store, with their subdomains", () => {
    expect(isBlocked("https://my.1password.com/vault", [])).toBe(true);
    expect(isBlocked("https://myaccount.google.com/security/signin", [])).toBe(true);
    expect(isBlocked("https://myaccount.google.com/personal-info", [])).toBe(false);
    expect(isBlocked("https://chromewebstore.google.com/detail/x", [])).toBe(true);
    expect(isBlocked("https://example.com/", [])).toBe(false);
  });

  it("can't be walked around with a final dot, doubled slashes, case or escapes", () => {
    expect(isBlocked("https://1password.com./", [])).toBe(true);
    expect(isBlocked("https://my.1password.com../vault", [])).toBe(true);
    expect(isBlocked("https://myaccount.google.com//security", [])).toBe(true);
    expect(isBlocked("https://myaccount.google.com/Security", [])).toBe(true);
    expect(isBlocked("https://MyAccount.Google.com/%73ecurity", [])).toBe(true);
    expect(isBlocked("https://myaccount.google.com/%E0%A4%A", [])).toBe(false);
    expect(isBlocked("blob:https://passwords.google.com/1234", [])).toBe(true);
    expect(isBlocked("https://online.mybank.com/Login", ["MyBank.com/login"])).toBe(true);
  });

  it("checks where a tab is and where it is going", () => {
    expect(isTabBlocked({ url: "https://example.com/", pendingUrl: "https://1password.com/" }, [])).toBe(true);
    expect(isTabBlocked({ url: "https://1password.com/" }, [])).toBe(true);
    expect(isTabBlocked({ url: "https://example.com/" }, [])).toBe(false);
  });

  it("blocks the owner's own sites, written either way", () => {
    expect(isBlocked("https://online.mybank.com/login", ["https://mybank.com"])).toBe(true);
    expect(isBlocked("https://mybank.com.evil.example/", ["mybank.com"])).toBe(false);
  });

  it("keeps it to web pages", () => {
    expect(isBlocked("chrome://settings", [])).toBe(true);
    expect(isBlocked("chrome-extension://abc/popup.html", [])).toBe(true);
    expect(isBlocked("file:///tmp/x.html", [])).toBe(true);
    expect(isBlocked("not a url", [])).toBe(true);
    expect(isBlocked("about:blank", [])).toBe(false);
    expect(isBlocked("", [])).toBe(false);
    expect(isBlocked("data:text/html,hi", [])).toBe(false);
  });

  it("names hosts, new-tab pages and the owner's list", () => {
    expect(hostOf("https://a.example.com:8443/x")).toBe("a.example.com:8443");
    expect(hostOf("nonsense")).toBe("nonsense");
    expect(isNewTabPage("chrome://newtab/")).toBe(true);
    expect(isNewTabPage("https://example.com")).toBe(false);
    expect(blockedList(" a.com \n\nb.com/x\n")).toEqual(["a.com", "b.com/x"]);
  });
});
