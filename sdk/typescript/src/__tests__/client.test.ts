/**
 * Tests for KnolClient
 */

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { KnolClient } from '../client.js';
import { KnolError } from '../types.js';

// Mock global fetch
const mockFetch = vi.fn();
vi.stubGlobal('fetch', mockFetch);

function jsonResponse(data: any, status = 200) {
  return Promise.resolve({
    ok: status >= 200 && status < 300,
    status,
    statusText: status === 200 ? 'OK' : 'Error',
    headers: new Headers({ 'content-type': 'application/json' }),
    json: () => Promise.resolve(data),
    text: () => Promise.resolve(JSON.stringify(data)),
  });
}

function emptyResponse(status = 204) {
  return Promise.resolve({
    ok: true,
    status,
    statusText: 'No Content',
    headers: new Headers({}),
    json: () => Promise.resolve(null),
    text: () => Promise.resolve(''),
  });
}

describe('KnolClient', () => {
  let client: KnolClient;

  beforeEach(() => {
    vi.clearAllMocks();
    client = new KnolClient({
      apiKey: 'test-api-key',
      baseUrl: 'https://api.test.com',
      retryAttempts: 1,
    });
  });

  describe('constructor', () => {
    it('should throw if apiKey is missing', () => {
      expect(() => new KnolClient({ apiKey: '' })).toThrow(KnolError);
    });

    it('should set defaults', () => {
      const c = new KnolClient({ apiKey: 'key' });
      expect(c).toBeDefined();
    });
  });

  describe('writeMemory', () => {
    it('should POST to /v1/memory', async () => {
      mockFetch.mockReturnValue(jsonResponse({ id: 'mem-1', content: 'test' }));

      const result = await client.writeMemory({ content: 'test', user_id: 'u1' });

      expect(mockFetch).toHaveBeenCalledOnce();
      const [url, init] = mockFetch.mock.calls[0];
      expect(url).toBe('https://api.test.com/v1/memory');
      expect(init.method).toBe('POST');
      expect(JSON.parse(init.body)).toEqual({ content: 'test', user_id: 'u1' });
      expect(result.id).toBe('mem-1');
    });
  });

  describe('getMemory', () => {
    it('should GET /v1/memory/:id', async () => {
      mockFetch.mockReturnValue(jsonResponse({ id: 'mem-1', content: 'hello' }));

      const result = await client.getMemory('mem-1');

      const [url] = mockFetch.mock.calls[0];
      expect(url).toBe('https://api.test.com/v1/memory/mem-1');
      expect(result.content).toBe('hello');
    });
  });

  describe('updateMemory', () => {
    it('should PUT /v1/memory/:id', async () => {
      mockFetch.mockReturnValue(jsonResponse({ id: 'mem-1' }));

      await client.updateMemory('mem-1', { content: 'updated' });

      const [url, init] = mockFetch.mock.calls[0];
      expect(url).toBe('https://api.test.com/v1/memory/mem-1');
      expect(init.method).toBe('PUT');
    });
  });

  describe('deleteMemory', () => {
    it('should DELETE /v1/memory/:id (soft delete)', async () => {
      mockFetch.mockReturnValue(emptyResponse());

      await client.deleteMemory('mem-1');

      const [url, init] = mockFetch.mock.calls[0];
      expect(url).toBe('https://api.test.com/v1/memory/mem-1');
      expect(init.method).toBe('DELETE');
    });

    it('should append ?permanent=true when permanent option is set', async () => {
      mockFetch.mockReturnValue(emptyResponse());

      await client.deleteMemory('mem-1', { permanent: true });

      const [url] = mockFetch.mock.calls[0];
      expect(url).toBe('https://api.test.com/v1/memory/mem-1?permanent=true');
    });
  });

  describe('restoreMemory', () => {
    it('should POST /v1/memory/:id/restore', async () => {
      mockFetch.mockReturnValue(jsonResponse({ id: 'mem-1', status: 'restored' }));

      const result = await client.restoreMemory('mem-1');

      const [url, init] = mockFetch.mock.calls[0];
      expect(url).toBe('https://api.test.com/v1/memory/mem-1/restore');
      expect(init.method).toBe('POST');
      expect(result.status).toBe('restored');
    });
  });

  describe('searchMemory', () => {
    it('should POST /v1/memory/search', async () => {
      mockFetch.mockReturnValue(
        jsonResponse({ results: [], total: 0, has_more: false })
      );

      await client.searchMemory({ query: 'test query', limit: 5 });

      const [url, init] = mockFetch.mock.calls[0];
      expect(url).toBe('https://api.test.com/v1/memory/search');
      expect(JSON.parse(init.body).query).toBe('test query');
    });
  });

  describe('SearchQueryBuilder', () => {
    it('should build a valid search request', async () => {
      mockFetch.mockReturnValue(
        jsonResponse({ results: [], total: 0, has_more: false })
      );

      const builder = client
        .searchBuilder()
        .query('test')
        .userId('u1')
        .limit(20)
        .tags(['tag1'])
        .minImportance(0.5)
        .applyDecay(true)
        .graphDepth(3);

      await client.searchMemory(builder);

      const body = JSON.parse(mockFetch.mock.calls[0][1].body);
      expect(body.query).toBe('test');
      expect(body.user_id).toBe('u1');
      expect(body.limit).toBe(20);
      expect(body.tags).toEqual(['tag1']);
      expect(body.min_importance).toBe(0.5);
      expect(body.apply_decay).toBe(true);
      expect(body.graph_depth).toBe(3);
    });

    it('should throw if query is empty', () => {
      expect(() => client.searchBuilder().build()).toThrow(KnolError);
    });
  });

  describe('graph operations', () => {
    it('listEntities should GET /v1/graph/entities', async () => {
      mockFetch.mockReturnValue(jsonResponse({ entities: [], total: 0 }));

      await client.listEntities('person', 50);

      const [url] = mockFetch.mock.calls[0];
      expect(url).toContain('/v1/graph/entities');
      expect(url).toContain('entity_type=person');
      expect(url).toContain('limit=50');
    });

    it('traverseGraph should GET /v1/graph/entities/:id/traverse', async () => {
      mockFetch.mockReturnValue(jsonResponse({ root: {}, paths: [] }));

      await client.traverseGraph('e1', 3, 50);

      const [url] = mockFetch.mock.calls[0];
      expect(url).toContain('/v1/graph/entities/e1/traverse');
      expect(url).toContain('depth=3');
    });

    it('findPath should GET /v1/graph/path/:from/:to', async () => {
      mockFetch.mockReturnValue(
        jsonResponse({ path: null, distance: 0, found: false })
      );

      await client.findPath('e1', 'e2', 6);

      const [url] = mockFetch.mock.calls[0];
      expect(url).toContain('/v1/graph/path/e1/e2');
      expect(url).toContain('max_depth=6');
    });
  });

  describe('error handling', () => {
    it('should throw KnolError on 4xx', async () => {
      mockFetch.mockReturnValue(
        jsonResponse({ error: 'Not Found', message: 'Memory not found' }, 404)
      );

      await expect(client.getMemory('bad-id')).rejects.toThrow(KnolError);
    });

    it('should not retry on 4xx', async () => {
      const retryClient = new KnolClient({
        apiKey: 'key',
        baseUrl: 'https://api.test.com',
        retryAttempts: 3,
      });

      mockFetch.mockReturnValue(
        jsonResponse({ message: 'bad request' }, 400)
      );

      await expect(retryClient.getMemory('bad')).rejects.toThrow(KnolError);
      expect(mockFetch).toHaveBeenCalledOnce();
    });

    it('should retry on 5xx', async () => {
      const retryClient = new KnolClient({
        apiKey: 'key',
        baseUrl: 'https://api.test.com',
        retryAttempts: 2,
        retryDelayMs: 1,
      });

      mockFetch
        .mockReturnValueOnce(jsonResponse({ message: 'Internal' }, 500))
        .mockReturnValueOnce(jsonResponse({ id: 'mem-1' }));

      const result = await retryClient.getMemory('mem-1');

      expect(mockFetch).toHaveBeenCalledTimes(2);
      expect(result.id).toBe('mem-1');
    });
  });

  describe('webhook operations', () => {
    it('should list webhooks', async () => {
      mockFetch.mockReturnValue(jsonResponse({ webhooks: [], total: 0 }));

      await client.listWebhooks();

      const [url] = mockFetch.mock.calls[0];
      expect(url).toBe('https://api.test.com/v1/webhooks');
    });

    it('should create webhook', async () => {
      mockFetch.mockReturnValue(jsonResponse({ id: 'wh-1' }));

      await client.createWebhook({
        url: 'https://example.com/hook',
        events: ['memory.created'],
      });

      const body = JSON.parse(mockFetch.mock.calls[0][1].body);
      expect(body.url).toBe('https://example.com/hook');
      expect(body.events).toEqual(['memory.created']);
    });

    it('should delete webhook', async () => {
      mockFetch.mockReturnValue(emptyResponse());

      await client.deleteWebhook('wh-1');

      const [url, init] = mockFetch.mock.calls[0];
      expect(url).toContain('/v1/webhooks/wh-1');
      expect(init.method).toBe('DELETE');
    });
  });

  describe('admin operations', () => {
    it('should get tenant usage', async () => {
      mockFetch.mockReturnValue(
        jsonResponse({ tenant_id: 't1', memory_count: 100 })
      );

      const result = await client.getTenantUsage();

      expect(result.tenant_id).toBe('t1');
    });

    it('should list audit log', async () => {
      mockFetch.mockReturnValue(
        jsonResponse({ entries: [], total: 0, has_more: false })
      );

      await client.listAuditLog({ limit: 50, offset: 10 });

      const [url] = mockFetch.mock.calls[0];
      expect(url).toContain('limit=50');
      expect(url).toContain('offset=10');
    });
  });
});
