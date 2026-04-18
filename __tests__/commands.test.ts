import { execute as helloExecute } from '../src/commands/hello';
import { execute as pingExecute } from '../src/commands/ping';
import { execute as helpExecute } from '../src/commands/help';

describe('Commands', () => {
  it('hello replies with Hello!', async () => {
    const mockMessage = {
      reply: jest.fn(),
      author: { tag: 'test#123' },
    } as any;
    await helloExecute(mockMessage);
    expect(mockMessage.reply).toHaveBeenCalledWith('Hello!');
  });

  it('ping replies with pong', async () => {
    const mockMessage = {
      reply: jest.fn(),
      author: { tag: 'test#123' },
    } as any;
    await pingExecute(mockMessage);
    expect(mockMessage.reply).toHaveBeenCalledWith('pong');
  });

  it('help replies with commands', async () => {
    const mockMessage = {
      reply: jest.fn(),
      author: { tag: 'test#123' },
    } as any;
    await helpExecute(mockMessage);
    expect(mockMessage.reply).toHaveBeenCalledWith('Available commands: hello, ping, help');
  });
});