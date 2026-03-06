import Tabs from '@theme/Tabs';
import TabItem from '@theme/TabItem';
import CodeBlock from '@theme/CodeBlock';
import React from 'react';

type Props = {
  pleaseCode: string;
  makeCode: string;
  justCode: string;
  pleaseLanguage?: string;
  makeLanguage?: string;
  justLanguage?: string;
};

export default function ToolComparison({
  pleaseCode,
  makeCode,
  justCode,
  pleaseLanguage = 'bash',
  makeLanguage = 'makefile',
  justLanguage = 'makefile',
}: Props): JSX.Element {
  return (
    <Tabs groupId="tool-compare" queryString>
      <TabItem value="please" label="Please">
        <CodeBlock language={pleaseLanguage}>{pleaseCode}</CodeBlock>
      </TabItem>
      <TabItem value="make" label="Make">
        <CodeBlock language={makeLanguage}>{makeCode}</CodeBlock>
      </TabItem>
      <TabItem value="just" label="Just">
        <CodeBlock language={justLanguage}>{justCode}</CodeBlock>
      </TabItem>
    </Tabs>
  );
}
