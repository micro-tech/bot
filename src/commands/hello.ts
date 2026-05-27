import { Message } from 'discord.js';
import logger from '../utils/logger';

export const name = 'hello';
export const execute = async (message: Message) => {
  try {
    await message.reply('Hello!');
    logger.info(`Responded to hello from ${message.author.tag}`);
  } catch (error) {
    logger.error(`Error responding to hello: ${error}`);
  }
};