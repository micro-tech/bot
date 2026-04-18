import { Message } from 'discord.js';
import logger from '../utils/logger';

export const name = 'ping';
export const execute = async (message: Message) => {
  try {
    await message.reply('pong');
    logger.info(`Responded to ping from ${message.author.tag}`);
  } catch (error) {
    logger.error(`Error responding to ping: ${error}`);
  }
};