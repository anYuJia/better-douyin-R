(() => {
    if (window.__dyRelationSignerProbeStarted) return;
    window.__dyRelationSignerProbeStarted = true;
    const save = (payload) => {
        try {
            const encoded = btoa(unescape(encodeURIComponent(JSON.stringify(payload))));
            document.cookie = `dy_relation_signer=${encodeURIComponent(encoded)}; domain=.douyin.com; path=/; max-age=600`;
            document.cookie = `dy_relation_signer=${encodeURIComponent(encoded)}; path=/; max-age=600`;
        } catch (error) {}
    };
    const readExistingSigner = () => {
        try {
            const match = document.cookie.match(/(?:^|; )dy_relation_signer=([^;]+)/);
            if (!match) return {};
            return JSON.parse(decodeURIComponent(escape(atob(decodeURIComponent(match[1])))));
        } catch (error) {
            return {};
        }
    };
    const bytesToBase64 = (value) => {
        const bytes = Array.from(value instanceof Uint8Array ? value : Object.values(value || {}));
        return btoa(String.fromCharCode(...bytes));
    };
    const looksLikeDtrait = (value) => String(value || "").trim().length > 20;
    const readStoredDtrait = () => {
        const direct = [window.__dtrait__, window.__dyRelationLatestDtrait];
        for (const value of direct) {
            if (looksLikeDtrait(value)) return String(value);
        }
        for (const storage of [window.localStorage, window.sessionStorage]) {
            try {
                if (!storage) continue;
                for (let index = 0; index < storage.length; index += 1) {
                    const key = storage.key(index);
                    const value = key ? storage.getItem(key) : "";
                    if (looksLikeDtrait(value)) return String(value);
                }
            } catch (error) {}
        }
        return "";
    };
    const findAwemeId = () => {
        try {
            const candidates = Array.from(document.querySelectorAll("a[href*='/video/']"))
                .map((node) => {
                    const href = node.getAttribute("href") || "";
                    const match = href.match(/\/video\/(\d+)/);
                    return match && match[1] || "";
                })
                .filter(Boolean);
            if (candidates.length > 0) return candidates[0];
        } catch (error) {}
        try {
            const html = document.documentElement && document.documentElement.innerHTML || "";
            const match = html.match(/"aweme_id"\s*:\s*"(\d{10,})"/) || html.match(/aweme_id=(\d{10,})/);
            return match && match[1] || "";
        } catch (error) {}
        return "";
    };
    const patchDtraitCapture = (onValue) => {
        window.__dyRelationDtraitListeners = window.__dyRelationDtraitListeners || [];
        if (typeof onValue === "function") {
            window.__dyRelationDtraitListeners.push(onValue);
            if (window.__dyRelationLatestDtrait) {
                try { onValue(window.__dyRelationLatestDtrait); } catch (error) {}
            }
        }
        if (window.__dyRelationDtraitPatched) return;
        window.__dyRelationDtraitPatched = true;
        const emit = (value) => {
            const text = String(value || "").trim();
            if (!text) return;
            window.__dyRelationLatestDtrait = text;
            for (const listener of window.__dyRelationDtraitListeners || []) {
                try { listener(text); } catch (error) {}
            }
        };
        try {
            const originalSetHeader = XMLHttpRequest.prototype.setRequestHeader;
            XMLHttpRequest.prototype.setRequestHeader = function(key, value) {
                if (String(key).toLowerCase() === "x-tt-session-dtrait" && value) {
                    emit(String(value));
                }
                return originalSetHeader.apply(this, arguments);
            };
        } catch (error) {}
        try {
            const originalFetch = window.fetch;
            window.fetch = function(input, init) {
                try {
                    const headers = init && init.headers;
                    let value = "";
                    if (headers && typeof headers.get === "function") {
                        value = headers.get("x-tt-session-dtrait") || "";
                    } else if (Array.isArray(headers)) {
                        const found = headers.find((item) => String(item && item[0]).toLowerCase() === "x-tt-session-dtrait");
                        value = found && found[1] || "";
                    } else if (headers && typeof headers === "object") {
                        value = headers["x-tt-session-dtrait"] || headers["X-Tt-Session-Dtrait"] || "";
                    }
                    if (!value && input && input.headers && typeof input.headers.get === "function") {
                        value = input.headers.get("x-tt-session-dtrait") || "";
                    }
                    if (value) emit(String(value));
                } catch (error) {}
                return originalFetch.apply(this, arguments);
            };
        } catch (error) {}
    };
    const captureDtrait = () => new Promise((resolve) => {
        let resolved = false;
        const finish = (value) => {
            if (resolved) return;
            resolved = true;
            resolve(value || "");
        };
        const stored = readStoredDtrait();
        if (stored) {
            finish(stored);
            return;
        }
        patchDtraitCapture(finish);
        try {
            const awemeId = findAwemeId() || "7640032041598198757";
            const xhr = new XMLHttpRequest();
            xhr.open("POST", "https://www-hj.douyin.com/aweme/v1/web/commit/item/digg/?device_platform=webapp&aid=6383&channel=channel_pc_web&pc_client_type=1&pc_libra_divert=Mac&update_version_code=170400&support_h265=1&support_dash=1&version_code=170400&version_name=17.4.0&cookie_enabled=true&browser_language=zh-CN&browser_platform=MacIntel&browser_name=Chrome&browser_version=148.0.0.0&browser_online=true&engine_name=Blink&engine_version=148.0.0.0&os_name=Mac%20OS&os_version=10.15.7&cpu_core_num=8&device_memory=16&platform=PC");
            xhr.setRequestHeader("Content-Type", "application/x-www-form-urlencoded; charset=UTF-8");
            xhr.setRequestHeader("x-secsdk-csrf-token", "DOWNGRADE");
            xhr.onloadend = () => setTimeout(() => finish(window.__dyRelationLatestDtrait || readStoredDtrait() || ""), 0);
            xhr.onerror = () => setTimeout(() => finish(window.__dyRelationLatestDtrait || readStoredDtrait() || ""), 0);
            xhr.send(`aweme_id=${awemeId}&item_type=0&type=0`);
        } catch (error) {
            finish(window.__dyRelationLatestDtrait || readStoredDtrait() || "");
        }
        setTimeout(() => finish(window.__dyRelationLatestDtrait || readStoredDtrait() || ""), 4000);
    });
    (async () => {
        try {
            const crypto = window.securitySDK && window.securitySDK.cryptoSDK;
            if (!crypto) throw new Error("security sdk not ready");
            const info = await crypto.getKeysInfoWithOrigin({ certType: "header", scene: "web_protect" });
            const ecdh = await crypto.initECDHKey();
            let privateKey = "";
            try {
                const storedCrypto = window.localStorage && window.localStorage.getItem("security-sdk/s_sdk_crypt_sdk") || "";
                const outer = storedCrypto ? JSON.parse(storedCrypto) : {};
                const inner = outer && outer.data ? JSON.parse(outer.data) : {};
                privateKey = inner && inner.ec_privateKey || "";
            } catch (error) {}
            const clientCert = info && info.sign && info.sign.client_cert || "";
            const existing = readExistingSigner();
            const payload = {
                ...existing,
                ticket: info && info.sign && info.sign.ticket || "",
                ts_sign: info && info.sign && info.sign.ts_sign || "",
                public_key: info && (info.b64PubKey || clientCert.replace(/^pub\./, "")) || "",
                client_cert: clientCert,
                private_key: privateKey || existing.private_key || "",
                ecdh_key: bytesToBase64(ecdh),
                uid: window.SSR_RENDER_DATA && window.SSR_RENDER_DATA.app && window.SSR_RENDER_DATA.app.odin && window.SSR_RENDER_DATA.app.odin.user_id || existing.uid || "",
                dtrait: existing.dtrait || "",
            };
            patchDtraitCapture((value) => {
                payload.dtrait = value || payload.dtrait;
                if (payload.ticket && payload.ts_sign && payload.public_key && payload.ecdh_key && payload.dtrait) save(payload);
            });
            payload.dtrait = await captureDtrait();
            if (payload.ticket && payload.ts_sign && payload.public_key && payload.ecdh_key) {
                save(payload);
                if (!payload.dtrait) window.__dyRelationSignerProbeStarted = false;
            } else {
                window.__dyRelationSignerProbeStarted = false;
            }
        } catch (error) {
            window.__dyRelationSignerProbeStarted = false;
        }
    })();
})();
