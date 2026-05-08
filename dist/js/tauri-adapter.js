/**
 * Tauri API 适配器 - 必须最先加载
 */

(function() {
    const debugLog = window.__DY_DEBUG__ ? console.log.bind(console) : () => {};
    debugLog('[Tauri Adapter] Script loaded, checking environment...');

    const adapterState = {
        fetchInstalled: false,
        mode: null,
        clientInitPromise: null,
        originalFetch: typeof window.fetch === 'function' ? window.fetch.bind(window) : null
    };

    const socketListeners = {};
    let socketId = 0;

    function clearLegacyCookieStorage() {
        try {
            localStorage.removeItem('cookie');
            const rawConfig = localStorage.getItem('dy_downloader_web_config');
            if (!rawConfig) return;

            const storedConfig = JSON.parse(rawConfig) || {};
            if (!Object.prototype.hasOwnProperty.call(storedConfig, 'cookie')) return;

            const hadCookie = Boolean(String(storedConfig.cookie || '').trim());
            delete storedConfig.cookie;
            storedConfig.cookie_set = Boolean(storedConfig.cookie_set || hadCookie);
            localStorage.setItem('dy_downloader_web_config', JSON.stringify(storedConfig));
        } catch (error) {}
    }

    clearLegacyCookieStorage();

    function jsonResponse(payload, status) {
        return new Response(JSON.stringify(payload), {
            status: status || 200,
            headers: { 'Content-Type': 'application/json' }
        });
    }

    function extractUrl(value) {
        if (!value) return '';

        if (typeof value === 'string') {
            return value.trim();
        }

        if (Array.isArray(value)) {
            for (let i = value.length - 1; i >= 0; i--) {
                const candidate = extractUrl(value[i]);
                if (candidate) return candidate;
            }
            return '';
        }

        if (typeof value === 'object') {
            if (typeof value.url === 'string' && value.url.trim()) {
                return value.url.trim();
            }
            if (Array.isArray(value.url_list)) {
                return extractUrl(value.url_list);
            }
            if (value.play_url) {
                return extractUrl(value.play_url);
            }
            if (value.play_addr) {
                return extractUrl(value.play_addr);
            }
        }

        return '';
    }

    function extractFirstUrl(value) {
        if (!value) return '';

        if (typeof value === 'string') {
            return value.trim();
        }

        if (Array.isArray(value)) {
            for (let i = 0; i < value.length; i++) {
                const candidate = extractFirstUrl(value[i]);
                if (candidate) return candidate;
            }
            return '';
        }

        if (typeof value === 'object') {
            if (typeof value.url === 'string' && value.url.trim()) {
                return value.url.trim();
            }
            if (Array.isArray(value.url_list)) {
                return extractFirstUrl(value.url_list);
            }
            if (value.play_url) {
                return extractFirstUrl(value.play_url);
            }
            if (value.play_addr) {
                return extractFirstUrl(value.play_addr);
            }
        }

        return '';
    }

    function inferMediaType(url, fallbackType) {
        const normalizedFallback = fallbackType === 'image' || fallbackType === 'live_photo' || fallbackType === 'mixed'
            ? fallbackType
            : 'video';

        if (!url || typeof url !== 'string') return normalizedFallback;

        const cleanUrl = url.split('?')[0].toLowerCase();
        if (/\.(jpg|jpeg|png|webp|gif|bmp|heic|heif)$/.test(cleanUrl)) return 'image';
        if (/\.(mp4|mov|m4v|webm|m3u8)$/.test(cleanUrl)) return 'video';
        if (cleanUrl.indexOf('/image') !== -1 || cleanUrl.indexOf('imagex') !== -1) return 'image';

        return normalizedFallback;
    }

    function dedupeMediaUrls(items) {
        const seen = new Set();
        return (items || []).filter(function(item) {
            if (!item || !item.url) return false;
            const key = item.type + '::' + item.url;
            if (seen.has(key)) return false;
            seen.add(key);
            return true;
        });
    }

    function normalizeUser(user) {
        const source = user && user.info ? Object.assign({}, user.info, user) : (user || {});
        return Object.assign({}, source, {
            avatar_thumb: source.avatar_thumb || source.avatar_medium || source.avatar_larger || '',
            avatar_medium: source.avatar_medium || source.avatar_thumb || source.avatar_larger || '',
            avatar_larger: source.avatar_larger || source.avatar_medium || source.avatar_thumb || '',
            total_favorited: source.total_favorited != null ? source.total_favorited : (source.favoriting_count || 0)
        });
    }

    function normalizeMusic(music, fallbackDuration) {
        const source = music || {};
        return Object.assign({}, source, {
            title: source.title || '',
            author: source.author || '',
            duration: Number(source.duration || fallbackDuration || 0),
            play_url: extractUrl(source.play_url)
        });
    }

    function resolveMediaType(video, mediaUrls) {
        if (video && (video.raw_media_type === 'video' || video.raw_media_type === 'image' || video.raw_media_type === 'live_photo' || video.raw_media_type === 'mixed')) {
            return video.raw_media_type;
        }
        if (video && (video.media_type === 'video' || video.media_type === 'image' || video.media_type === 'live_photo' || video.media_type === 'mixed')) {
            return video.media_type;
        }
        if (video && video.is_image) return 'image';

        const hasVideo = (mediaUrls || []).some(function(item) { return item.type === 'video' || item.type === 'live_photo'; });
        const hasImage = (mediaUrls || []).some(function(item) { return item.type === 'image'; });

        if (hasVideo && hasImage) return 'mixed';
        if (hasImage) return 'image';
        return 'video';
    }

    function normalizeVideo(video) {
        if (!video || typeof video !== 'object') return video;

        const author = normalizeUser(video.author || video.user || {});
        const stats = video.statistics || {};
        const cover = video.cover_url || (video.video && video.video.cover) || video.cover || '';
        const playAddr = extractFirstUrl((video.video && video.video.play_addr) || video.play_addr);
        const previewAddr = extractFirstUrl(
            (video.video && (video.video.preview_addr || video.video.play_addr_lowbr || video.video.play_addr_h264))
            || video.preview_addr
            || video.play_addr_lowbr
            || video.play_addr_h264
        ) || playAddr;
        const fallbackType = video.is_image ? 'image' : 'video';
        const mediaUrls = [];

        if (Array.isArray(video.media_urls)) {
            video.media_urls.forEach(function(item) {
                if (typeof item === 'string') {
                    const url = item.trim();
                    if (url) mediaUrls.push({ type: inferMediaType(url, fallbackType), url: url });
                    return;
                }

                if (item && typeof item === 'object') {
                    const url = extractUrl(item.url || item.url_list || item.play_url || item.play_addr || item);
                    if (!url) return;
                    mediaUrls.push({
                        type: item.type || inferMediaType(url, fallbackType),
                        url: url
                    });
                }
            });
        }

        if (previewAddr) {
            mediaUrls.unshift({ type: fallbackType === 'image' ? 'image' : 'video', url: previewAddr });
        }

        if (Array.isArray(video.image_urls)) {
            video.image_urls.forEach(function(url) {
                const normalizedUrl = extractUrl(url);
                if (normalizedUrl) mediaUrls.push({ type: 'image', url: normalizedUrl });
            });
        }

        if (Array.isArray(video.images)) {
            video.images.forEach(function(item) {
                const normalizedUrl = extractUrl(item);
                if (normalizedUrl) mediaUrls.push({ type: 'image', url: normalizedUrl });
            });
        }

        if (video.video && Array.isArray(video.video.images)) {
            video.video.images.forEach(function(item) {
                const normalizedUrl = extractUrl(item);
                if (normalizedUrl) mediaUrls.push({ type: 'image', url: normalizedUrl });
            });
        }

        const normalizedMediaUrls = dedupeMediaUrls(mediaUrls);
        const mediaType = resolveMediaType(video, normalizedMediaUrls);
        const normalizedMusic = normalizeMusic(video.music, (video.video && video.video.duration) || video.duration || 0);
        const duration = Number(video.duration || (video.video && video.video.duration) || normalizedMusic.duration || 0);
        const imageUrls = normalizedMediaUrls
            .filter(function(item) { return item.type === 'image'; })
            .map(function(item) { return item.url; });

        return Object.assign({}, video, {
            author: author,
            digg_count: video.digg_count != null ? video.digg_count : (stats.digg_count || 0),
            comment_count: video.comment_count != null ? video.comment_count : (stats.comment_count || 0),
            share_count: video.share_count != null ? video.share_count : (stats.share_count || 0),
            cover: cover,
            cover_url: cover,
            preview_addr: previewAddr,
            raw_media_type: mediaType,
            media_type: mediaType,
            media_urls: normalizedMediaUrls,
            duration: duration,
            statistics: {
                digg_count: video.digg_count != null ? video.digg_count : (stats.digg_count || 0),
                comment_count: video.comment_count != null ? video.comment_count : (stats.comment_count || 0),
                share_count: video.share_count != null ? video.share_count : (stats.share_count || 0),
                play_count: video.play_count != null ? video.play_count : (stats.play_count || 0)
            },
            video: Object.assign({}, video.video || {}, {
                preview_addr: previewAddr,
                play_addr: playAddr,
                cover: cover,
                duration: duration,
                images: imageUrls,
                media_urls: normalizedMediaUrls
            }),
            music: normalizedMusic,
            bgm_url: normalizedMusic.play_url || video.bgm_url || video.music_url || ''
        });
    }

    function normalizeVideos(videos) {
        return Array.isArray(videos) ? videos.map(normalizeVideo) : [];
    }

    function normalizeHistoryItem(item) {
        const path = item.file_path || item.path || '';
        const name = path.split(/[\\/]/).pop() || item.title || '未命名文件';
        return {
            aweme_id: item.aweme_id || '',
            path: path,
            file_path: path,
            name: name,
            title: item.title || name,
            author: item.author || '',
            cover: item.cover || '',
            size: Number(item.file_size || item.size || 0),
            file_size: Number(item.file_size || item.size || 0),
            modified_at: Number(item.create_time || item.modified_at || 0),
            create_time: Number(item.create_time || 0),
            media_type: item.media_type || ''
        };
    }

    function readWebPreviewConfig() {
        let storedConfig = {};

        try {
            storedConfig = JSON.parse(localStorage.getItem('dy_downloader_web_config') || '{}') || {};
        } catch (error) {
            storedConfig = {};
        }

        return Object.assign({}, storedConfig, {
            download_dir: storedConfig.download_dir || storedConfig.download_path || '',
            download_path: storedConfig.download_path || storedConfig.download_dir || '',
            cookie: '',
            max_concurrent: normalizePositiveInteger(storedConfig.max_concurrent, 3),
            cookie_set: Boolean(storedConfig.cookie_set)
        });
    }

    function saveWebPreviewConfig(partialConfig) {
        const nextConfig = Object.assign({}, readWebPreviewConfig(), partialConfig || {});
        if (partialConfig && partialConfig.download_dir) {
            nextConfig.download_path = partialConfig.download_dir;
        }

        nextConfig.download_dir = nextConfig.download_dir || nextConfig.download_path || '';
        nextConfig.download_path = nextConfig.download_path || nextConfig.download_dir || '';
        nextConfig.max_concurrent = normalizePositiveInteger(nextConfig.max_concurrent, 3);
        nextConfig.cookie_set = Boolean(nextConfig.cookie_set || (partialConfig && partialConfig.cookie));

        localStorage.setItem('dy_downloader_web_config', JSON.stringify({
            download_dir: nextConfig.download_dir,
            download_path: nextConfig.download_path,
            cookie_set: nextConfig.cookie_set,
            max_concurrent: nextConfig.max_concurrent
        }));

        return nextConfig;
    }

    function webPreviewMessage() {
        return '当前页面运行在浏览器静态预览模式，请通过 Tauri 桌面应用启动本项目。';
    }

    function normalizePositiveInteger(value, fallbackValue) {
        const parsed = Number(value);
        if (!Number.isFinite(parsed) || parsed <= 0) {
            return fallbackValue;
        }
        return Math.floor(parsed);
    }

    function normalizeConfigForCommand(input, baseConfig) {
        const base = Object.assign({}, baseConfig || {});
        const rawInput = Object.assign({}, input || {});
        const fallbackMaxConcurrent = normalizePositiveInteger(base.max_concurrent, 3);
        const config = Object.assign({}, base, rawInput);

        if (rawInput.download_dir) {
            config.download_path = rawInput.download_dir;
        } else if (rawInput.download_path) {
            config.download_path = rawInput.download_path;
        }

        config.download_path = config.download_path || base.download_path || base.download_dir || '';
        config.cookie = typeof config.cookie === 'string'
            ? config.cookie
            : (typeof base.cookie === 'string' ? base.cookie : '');
        config.max_concurrent = normalizePositiveInteger(config.max_concurrent, fallbackMaxConcurrent);

        delete config.download_dir;
        delete config.cookie_set;

        return config;
    }

    function resolveTauriBridge() {
        if (window.__TAURI__ && window.__TAURI__.core && typeof window.__TAURI__.core.invoke === 'function') {
            return window.__TAURI__;
        }

        const internals = window.__TAURI_INTERNALS__;
        if (!internals || typeof internals.invoke !== 'function') {
            return null;
        }

        function listen(eventName, callback, options) {
            const target = typeof (options && options.target) === 'string'
                ? { kind: 'AnyLabel', label: options.target }
                : ((options && options.target) || { kind: 'Any' });

            const handlerId = typeof internals.transformCallback === 'function'
                ? internals.transformCallback(callback)
                : null;

            if (!handlerId) {
                return Promise.reject(new Error('Tauri event bridge unavailable'));
            }

            return internals.invoke('plugin:event|listen', {
                event: eventName,
                target: target,
                handler: handlerId
            }).then(function(eventId) {
                return function() {
                    try {
                        if (window.__TAURI_EVENT_PLUGIN_INTERNALS__ && typeof window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener === 'function') {
                            window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener(eventName, eventId);
                        }
                    } catch (error) {}

                    return internals.invoke('plugin:event|unlisten', {
                        event: eventName,
                        eventId: eventId
                    }).catch(function() {});
                };
            });
        }

        window.__TAURI__ = {
            core: {
                invoke: internals.invoke.bind(internals)
            },
            event: {
                listen: listen
            }
        };

        return window.__TAURI__;
    }

    function updateEnvironmentState() {
        const bridge = resolveTauriBridge();
        const mode = bridge ? 'tauri' : 'web-preview';

        window.DY_ENV = {
            isTauri: !!bridge,
            apiMode: mode,
            supportsNativeApi: !!bridge
        };

        if (adapterState.mode !== mode) {
            debugLog('[Tauri Adapter] Environment mode:', mode);
            adapterState.mode = mode;
        }

        return bridge;
    }

    function ensureClientReady(bridge) {
        if (!bridge) return Promise.resolve();

        if (!adapterState.clientInitPromise) {
            adapterState.clientInitPromise = bridge.core.invoke('init_client')
                .then(function(result) {
                    debugLog('[Tauri Adapter] Client initialized');
                    return result;
                })
                .catch(function(error) {
                    adapterState.clientInitPromise = null;
                    throw error;
                });
        }

        return adapterState.clientInitPromise;
    }

    async function handleTauriApiRequest(path, method, params, body, bridge) {
        const invoke = bridge.core.invoke;
        const needsClient = path !== '/api/config' &&
            path !== '/api/select_directory' &&
            path !== '/api/download_history' &&
            path !== '/api/download_history/delete' &&
            path !== '/api/download_history/open' &&
            path !== '/api/download_history/open_directory' &&
            path !== '/api/download_history/open_location' &&
            path !== '/api/get_app_version' &&
            path !== '/api/check_update' &&
            path !== '/api/download_update' &&
            path !== '/api/restart_app' &&
            path !== '/api/cookie/generate_temp';

        if (needsClient) {
            await ensureClientReady(bridge);
        }

        if (path === '/api/config' && method === 'GET') {
            const config = await invoke('get_config');
            return Object.assign({}, config || {}, {
                download_dir: (config && (config.download_dir || config.download_path)) || '',
                download_path: (config && (config.download_path || config.download_dir)) || '',
                download_quality: (config && config.download_quality) || 'auto',
                max_concurrent: (config && config.max_concurrent) || 3,
                cookie: '',
                cookie_set: Boolean(config && (config.cookie_set || (config.cookie && String(config.cookie).trim())))
            });
        }

        if (path === '/api/config' && method === 'POST') {
            const existingConfig = await invoke('get_config').catch(function() { return {}; });
            const configBody = normalizeConfigForCommand(body, existingConfig);

            const result = await invoke('save_config', { config: configBody });
            adapterState.clientInitPromise = null;
            await ensureClientReady(bridge);
            return Object.assign({
                success: true,
                message: '配置保存成功'
            }, result || {});
        }

        if (path === '/api/select_directory') {
            const selectedPath = await invoke('select_directory');
            if (selectedPath) {
                return { success: true, path: selectedPath };
            }
            return { success: false, path: null, message: '用户取消选择' };
        }

        if (path === '/api/search_user') {
            const result = await invoke('search_user', {
                keyword: body.keyword || params.keyword || ''
            });

            if (result && result.success === false) {
                return Object.assign({ success: false }, result);
            }

            if (result && result.type === 'single' && result.user) {
                const user = normalizeUser(result.user);
                return {
                    success: true,
                    type: 'single',
                    user: user,
                    users: user ? [user] : []
                };
            }

            if (result && result.type === 'multiple' && Array.isArray(result.users)) {
                return {
                    success: true,
                    type: 'multiple',
                    users: result.users.map(normalizeUser)
                };
            }

            const normalizedUsers = Array.isArray(result) ? result.map(normalizeUser) : [];

            if (normalizedUsers.length === 0) {
                return { success: false, message: '未找到匹配用户' };
            }

            if (normalizedUsers.length === 1) {
                return {
                    success: true,
                    type: 'single',
                    user: normalizedUsers[0],
                    users: normalizedUsers
                };
            }

            return {
                success: true,
                type: 'multiple',
                users: normalizedUsers
            };
        }

        if (path === '/api/user_detail') {
            const result = await invoke('get_user_detail', {
                secUid: body.sec_uid || body.secUid || params.sec_uid || params.secUid || '',
                nickname: body.nickname || params.nickname || null
            });

            if (result && result.success === false) {
                return Object.assign({ success: false }, result);
            }

            const user = result && result.user ? result.user : result;

            return {
                success: true,
                user: normalizeUser(user)
            };
        }

        if (path === '/api/user_videos') {
            const result = await invoke('get_user_videos', {
                secUid: body.sec_uid || body.secUid || params.sec_uid || params.secUid || '',
                cursor: Number(body.cursor || params.cursor || 0),
                count: Number(body.count || params.count || 20)
            });

            return {
                success: true,
                videos: normalizeVideos(result && result.videos),
                cursor: Number(result && result.cursor) || 0,
                has_more: Boolean(result && result.has_more)
            };
        }

        if (path === '/api/video_detail') {
            const result = await invoke('get_video_detail', {
                awemeId: body.aweme_id || body.awemeId || params.aweme_id || params.awemeId || ''
            });

            if (result && result.success === false) {
                return Object.assign({ success: false }, result);
            }

            const video = result && result.video ? result.video : result;

            return {
                success: true,
                video: normalizeVideo(video)
            };
        }

        if (path === '/api/parse_link') {
            const result = await invoke('parse_link', {
                link: body.link || params.link || ''
            });

            if (result && result.success === false) {
                return Object.assign({ success: false }, result);
            }

            const sourceVideo = result && result.video ? result.video : result;
            const normalizedVideo = normalizeVideo(sourceVideo);

            return {
                success: true,
                type: (result && result.type) || 'video',
                videos: normalizedVideo ? [normalizedVideo] : [],
                video: normalizedVideo,
                user: result && result.user ? normalizeUser(result.user) : undefined
            };
        }

        if (path === '/api/recommended_feed') {
            const result = await invoke('get_recommended', {
                cursor: Number(body.cursor || params.cursor || 0),
                count: Number(body.count || params.count || 20)
            });

            return {
                success: true,
                videos: normalizeVideos(result && result.videos),
                cursor: Number(result && result.cursor) || 0,
                has_more: Boolean(result && result.has_more)
            };
        }

        if (path === '/api/download_single_video') {
            const result = await invoke('download_video', { video: body || {} });
            return Object.assign({
                success: true,
                message: '已添加下载任务'
            }, result || {});
        }

        if (path === '/api/download_user_video') {
            const result = await invoke('download_user_videos', {
                secUid: body.sec_uid || body.secUid || '',
                nickname: body.nickname || '',
                awemeCount: Number(body.aweme_count || body.awemeCount || 0)
            });
            return Object.assign({}, result || {});
        }

        if (path === '/api/get_liked_videos') {
            const result = await invoke('get_liked_videos', {
                secUid: '',
                cursor: 0,
                count: Number(body.count || params.count || 20)
            });
            const rawVideos = result && (result.data || result.videos);
            const videos = normalizeVideos(rawVideos);

            if (!Array.isArray(videos) || videos.length === 0) {
                return {
                    success: false,
                    message: (result && result.message) || '获取点赞视频失败。该接口需要登录态，请确认Cookie有效且包含完整的登录信息。如果Cookie已过期请重新获取。'
                };
            }

            return {
                success: true,
                data: videos,
                count: Number(result && result.count) || videos.length,
                cursor: Number(result && result.cursor) || 0,
                has_more: Boolean(result && result.has_more)
            };
        }

        if (path === '/api/get_liked_authors') {
            const result = await invoke('get_liked_authors', {
                count: Number(body.count || params.count || 20)
            });

            if (result && result.success === false) {
                return {
                    success: false,
                    message: result.message || '获取点赞作者失败。该接口需要登录态，请确认Cookie有效且包含完整的登录信息。'
                };
            }

            const detailedAuthors = Array.isArray(result && result.data)
                ? result.data.map(normalizeUser)
                : [];

            return {
                success: true,
                data: detailedAuthors,
                count: Number(result && result.count) || detailedAuthors.length
            };
        }

        if (path === '/api/download_liked') {
            return invoke('download_liked_videos', {
                count: Number(body.count || params.count || 20)
            });
        }

        if (path === '/api/get_collected_videos') {
            console.log('[Adapter] get_collected_videos request:', { cursor: Number(body.cursor || params.cursor || 0), count: Number(body.count || params.count || 20) });
            const result = await invoke('get_collected_videos', {
                cursor: Number(body.cursor || params.cursor || 0),
                count: Number(body.count || params.count || 20)
            });
            console.log('[Adapter] get_collected_videos response:', result);

            if (result && result.success === false) {
                console.error('[Adapter] get_collected_videos error:', result.message);
                return {
                    success: false,
                    message: (result && result.message) || '获取收藏视频失败。该接口需要登录态，请确认Cookie有效且包含完整的登录信息。如果Cookie已过期请重新获取。',
                    has_more: false,
                    next_cursor: 0
                };
            }

            const rawVideos = result && (result.data || result.videos);
            const videos = normalizeVideos(rawVideos);

            return {
                success: true,
                data: videos || [],
                count: Number(result && result.count) || (videos && videos.length) || 0,
                has_more: Boolean(result && result.has_more),
                next_cursor: Number(result && result.next_cursor) || 0
            };
        }

        if (path === '/api/get_collected_mixes') {
            console.log('[Adapter] get_collected_mixes request:', { cursor: Number(body.cursor || params.cursor || 0), count: Number(body.count || params.count || 20) });
            const result = await invoke('get_collected_mixes', {
                cursor: Number(body.cursor || params.cursor || 0),
                count: Number(body.count || params.count || 20)
            });
            console.log('[Adapter] get_collected_mixes response:', result);

            if (result && result.success === false) {
                console.error('[Adapter] get_collected_mixes error:', result.message);
                return {
                    success: false,
                    message: (result && result.message) || '获取收藏合集失败。该接口需要登录态，请确认Cookie有效且包含完整的登录信息。',
                    has_more: false,
                    next_cursor: 0
                };
            }

            const mixes = result && result.data;

            return {
                success: true,
                data: mixes || [],
                count: Number(result && result.count) || (mixes && mixes.length) || 0,
                has_more: Boolean(result && result.has_more),
                next_cursor: Number(result && result.next_cursor) || 0
            };
        }

        if (path === '/api/get_mix_videos') {
            console.log('[Adapter] get_mix_videos request:', { seriesId: body.series_id || body.seriesId || params.series_id || params.seriesId || '', cursor: Number(body.cursor || params.cursor || 0), count: Number(body.count || params.count || 12) });
            const result = await invoke('get_mix_videos', {
                seriesId: body.series_id || body.seriesId || params.series_id || params.seriesId || '',
                cursor: Number(body.cursor || params.cursor || 0),
                count: Number(body.count || params.count || 12)
            });
            console.log('[Adapter] get_mix_videos response:', result);

            if (result && result.success === false) {
                console.error('[Adapter] get_mix_videos error:', result.message);
                return {
                    success: false,
                    message: (result && result.message) || '获取合集视频失败。',
                    has_more: false,
                    next_cursor: 0
                };
            }

            const videos = normalizeVideos(result && result.data);

            return {
                success: true,
                data: videos,
                count: Number(result && result.count) || videos.length,
                has_more: Boolean(result && result.has_more),
                next_cursor: Number(result && result.next_cursor) || 0
            };
        }

        if (path === '/api/download_collected') {
            return invoke('download_collected_videos', {
                count: Number(body.count || params.count || 20)
            });
        }

        if (path === '/api/cancel_download') {
            await invoke('cancel_download_task', {
                taskId: body.task_id || body.taskId || ''
            });
            return { success: true, message: '已取消下载任务' };
        }

        if (path === '/api/pause_download') {
            await invoke('pause_download', {
                taskId: body.task_id || body.taskId || ''
            });
            return { success: true, message: '已暂停下载任务' };
        }

        if (path === '/api/resume_download') {
            await invoke('resume_download', {
                taskId: body.task_id || body.taskId || ''
            });
            return { success: true, message: '已恢复下载任务' };
        }

        if (path === '/api/download_history') {
            const historyResult = await invoke('get_history');
            const config = await invoke('get_config').catch(function() { return {}; });
            return {
                success: true,
                items: Array.isArray(historyResult && historyResult.items) ? historyResult.items.map(normalizeHistoryItem) : (Array.isArray(historyResult) ? historyResult.map(normalizeHistoryItem) : []),
                download_root: (config && (config.download_path || config.download_dir)) || ''
            };
        }

        if (path === '/api/download_history/open') {
            const targetPath = body.path || body.file_path || '';
            if (targetPath) {
                await invoke('open_file', { path: targetPath });
            } else {
                await invoke('open_download_directory');
            }
            return { success: true };
        }

        if (path === '/api/download_history/open_location') {
            const targetPath = body.path || body.file_path || '';
            if (targetPath) {
                await invoke('open_file_location', { path: targetPath });
            } else {
                await invoke('open_download_directory');
            }
            return { success: true };
        }

        if (path === '/api/download_history/open_directory') {
            await invoke('open_download_directory');
            return { success: true };
        }

        if (path === '/api/download_history/delete') {
            const paths = Array.isArray(body.paths) ? body.paths : [];

            if (paths.length > 0) {
                const historyResult = await invoke('get_history').catch(function() { return []; });
                const historyItems = Array.isArray(historyResult && historyResult.items) ? historyResult.items : (Array.isArray(historyResult) ? historyResult : []);
                let deletedCount = 0;

                for (let i = 0; i < paths.length; i++) {
                    const targetPath = paths[i];

                    try {
                        await invoke('delete_file', { path: targetPath });
                    } catch (error) {}

                    const historyMatch = Array.isArray(historyItems)
                        ? historyItems.find(function(item) { return item && item.file_path === targetPath; })
                        : null;

                    if (historyMatch && historyMatch.aweme_id) {
                        try {
                            await invoke('delete_history', { awemeId: historyMatch.aweme_id });
                        } catch (error) {}
                    }

                    deletedCount += 1;
                }

                return {
                    success: true,
                    deleted_count: deletedCount
                };
            }

            await invoke('delete_history', {
                awemeId: body.aweme_id || body.awemeId || ''
            });

            return { success: true, deleted_count: 1 };
        }

        if (path === '/api/cookie/browser_login') {
            return invoke('cookie_browser_login', {
                timeout: Number(body.timeout || 300),
                browser: body.browser || 'chrome'
            });
        }

        if (path === '/api/cookie/browser_login/cancel') {
            return invoke('cancel_cookie_browser_login');
        }

        if (path === '/api/cookie/from_browser') {
            return { success: false, message: 'Tauri 版本暂不支持自动读取浏览器 Cookie，请手动复制。' };
        }

        if (path === '/api/cookie/generate_temp') {
            const tempCookie = 'ttwid=1%7C' + Date.now() + '; s_v_web_id=verify_' + Math.random().toString(36).slice(2, 11);
            return { success: true, cookie: tempCookie };
        }

        if (path === '/api/tasks') {
            const result = await invoke('get_download_tasks');
            return { success: true, tasks: (result && result.tasks) || (Array.isArray(result) ? result : []) };
        }

        if (path === '/api/open_verify_browser') {
            return invoke('open_verify_browser', {
                targetUrl: body.target_url || body.targetUrl || ''
            });
        }

        if (path === '/api/get_app_version') {
            return invoke('get_app_version');
        }

        if (path === '/api/check_update') {
            return invoke('check_update');
        }

        if (path === '/api/download_update') {
            return invoke('download_update');
        }

        if (path === '/api/restart_app') {
            return invoke('restart_app');
        }

        if (path === '/api/download_music') {
            return { success: false, message: '请直接使用媒体原始地址下载音乐。' };
        }

        if (path === '/api/read_clipboard') {
            const result = await invoke('read_clipboard');
            return { success: true, text: result || '' };
        }

        console.warn('[Tauri Adapter] Unmatched API:', method, path);
        return {
            success: false,
            message: '未实现的 API: ' + path
        };
    }

    async function handleWebPreviewApiRequest(path, method, params, body) {
        if (path === '/api/config' && method === 'GET') {
            return readWebPreviewConfig();
        }

        if (path === '/api/config' && method === 'POST') {
            saveWebPreviewConfig(body || {});
            return { success: true, message: '已保存到浏览器本地存储' };
        }

        if (path === '/api/cookie/generate_temp') {
            const tempCookie = 'ttwid=1%7C' + Date.now() + '; s_v_web_id=verify_' + Math.random().toString(36).slice(2, 11);
            return { success: true, cookie: tempCookie };
        }

        if (path === '/api/select_directory') {
            return { success: false, message: '浏览器预览模式不支持选择本地目录。', path: null };
        }

        if (path === '/api/recommended_feed') {
            return {
                success: false,
                message: webPreviewMessage(),
                videos: [],
                cursor: 0,
                has_more: false
            };
        }

        if (path === '/api/download_history') {
            return {
                success: false,
                message: webPreviewMessage(),
                items: [],
                download_root: ''
            };
        }

        return {
            success: false,
            message: webPreviewMessage()
        };
    }

    function parseApiRequest(input, init) {
        const method = ((init && init.method) || 'GET').toUpperCase();
        const rawUrl = typeof input === 'string' ? input : (input && input.url) || '';
        const urlObject = new URL(rawUrl, window.location.origin);
        const path = urlObject.pathname;
        const params = Object.fromEntries(urlObject.searchParams.entries());
        let body = {};

        if (init && init.body) {
            try {
                body = typeof init.body === 'string' ? JSON.parse(init.body) : init.body;
            } catch (error) {
                body = {};
            }
        }

        return {
            method: method,
            rawUrl: rawUrl,
            origin: urlObject.origin,
            path: path,
            params: params,
            body: body
        };
    }

    function installFetchInterceptor() {
        if (adapterState.fetchInstalled || !adapterState.originalFetch) return;

        adapterState.fetchInstalled = true;

        window.fetch = async function(input, init) {
            const request = parseApiRequest(input, init || {});

            if (request.path.indexOf('/api/media/proxy') === 0) {
                return adapterState.originalFetch(input, init);
            }

            if (request.path.indexOf('/api/') !== 0) {
                return adapterState.originalFetch(input, init);
            }

            const bridge = updateEnvironmentState();
            debugLog('[Tauri Adapter] API request:', bridge ? 'tauri' : 'web-preview', request.method, request.path);

            try {
                const result = bridge
                    ? await handleTauriApiRequest(request.path, request.method, request.params, request.body, bridge)
                    : await handleWebPreviewApiRequest(request.path, request.method, request.params, request.body);

                return jsonResponse(result, 200);
            } catch (error) {
                console.error('[Tauri Adapter] API error:', request.path, error);
                return jsonResponse({
                    success: false,
                    message: error && error.message ? error.message : String(error)
                }, 500);
            }
        };
    }

    window.io = function() {
        debugLog('[Socket Mock] io() called');
        socketId += 1;

        const socket = {
            id: 'socket_' + socketId,
            connected: false,

            on: function(event, callback) {
                if (!socketListeners[event]) socketListeners[event] = [];
                socketListeners[event].push(callback);

                const bridge = resolveTauriBridge();
                const eventMap = {
                    download_progress: 'download-progress',
                    download_started: 'download-started',
                    download_log: 'download-log',
                    download_completed: 'download-completed',
                    download_failed: 'download-failed',
                    download_error: 'download-error',
                    download_success: 'download-success',
                    download_info: 'download-info',
                    user_video_download_progress: 'user-video-download-progress',
                    user_video_download_failed: 'user-video-download-failed',
                    current_video_progress: 'current-video-progress',
                    batch_download_started: 'batch-download-started',
                    batch_download_completed: 'batch-download-completed',
                    batch_download_cancelled: 'batch-download-cancelled',
                    log: 'log',
                    connect: 'connect',
                    disconnect: 'disconnect',
                    cookie_login_status: 'cookie-login-status'
                };

                if (bridge && bridge.event && typeof bridge.event.listen === 'function' && eventMap[event]) {
                    bridge.event.listen(eventMap[event], function(e) {
                        callback(e && e.payload !== undefined ? e.payload : e);
                    }).catch(function(error) {
                        console.warn('[Socket Mock] Failed to bind Tauri event:', event, error);
                    });
                }

                return this;
            },

            emit: function(event, data) {
                const bridge = resolveTauriBridge();

                if (bridge && event === 'get_config') {
                    bridge.core.invoke('get_config').then(function(config) {
                        socket._trigger('config', config);
                    });
                } else if (bridge && event === 'save_config') {
                    bridge.core.invoke('get_config')
                        .catch(function() { return {}; })
                        .then(function(config) {
                            return bridge.core.invoke('save_config', {
                                config: normalizeConfigForCommand(data, config)
                            });
                        })
                        .then(function() {
                            socket._trigger('config_saved', { success: true });
                        });
                }

                return this;
            },

            off: function(event) {
                socketListeners[event] = [];
                return this;
            },

            disconnect: function() {
                this.connected = false;
            },

            connect: function() {
                this.connected = true;
                this._trigger('connect');
            },

            _trigger: function(event, data) {
                if (!socketListeners[event]) return;
                socketListeners[event].forEach(function(cb) {
                    cb(data);
                });
            }
        };

        setTimeout(function() {
            socket.connect();
        }, 100);

        return socket;
    };

    debugLog('[Tauri Adapter] window.io defined');

    installFetchInterceptor();
    updateEnvironmentState();

    document.addEventListener('DOMContentLoaded', function() {
        debugLog('[Tauri Adapter] DOM ready, checking __TAURI__:', typeof window.__TAURI__, 'internals:', typeof window.__TAURI_INTERNALS__);
        updateEnvironmentState();

        setTimeout(function() {
            debugLog('[Tauri Adapter] After timeout, __TAURI__:', typeof window.__TAURI__, 'internals:', typeof window.__TAURI_INTERNALS__);
            updateEnvironmentState();
        }, 100);
    });
})();
